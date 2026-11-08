//! Saved monitor profiles: one TOML file per profile in the profiles folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hotkey::Hotkey;
use crate::model::DisplayLayout;
use crate::paths::write_atomic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<Hotkey>,
    #[serde(flatten)]
    pub layout: DisplayLayout,
}

impl Profile {
    pub fn new(name: impl Into<String>, layout: DisplayLayout) -> Result<Self> {
        let name = name.into().trim().to_string();
        validate_name(&name)?;
        Ok(Profile {
            name,
            hotkey: None,
            layout,
        })
    }
}

/// Names Windows reserves for devices: `con.toml` can't be a file there.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Profile names become file names, so keep them simple.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && !RESERVED.contains(&slug(name).as_str())
        && name.chars().count() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        && name.chars().any(|c| c.is_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidProfileName(name.to_string()))
    }
}

/// File-name slug: lowercase, spaces to '-'. Names that differ only in case
/// or spacing map to the same file, so they count as the same profile.
pub fn slug(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ProfileStore { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.toml", slug(name)))
    }

    /// All readable profiles, sorted by name. Files that can't be read
    /// (e.g. a hand edit gone wrong) are skipped; see [`Self::list_checked`]
    /// to find out about them.
    pub fn list(&self) -> Result<Vec<Profile>> {
        Ok(self.list_checked()?.0)
    }

    /// All readable profiles sorted by name, plus one error per profile file
    /// that couldn't be read. One broken file must not take every other
    /// profile (and its hotkey) down with it.
    pub fn list_checked(&self) -> Result<(Vec<Profile>, Vec<Error>)> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
            Err(e) => return Err(Error::io(&self.dir, e)),
        };
        let mut profiles = Vec::new();
        let mut problems = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| Error::io(&self.dir, e))?.path();
            if path.extension().is_some_and(|x| x == "toml") {
                match read_profile(&path) {
                    Ok(p) => profiles.push(p),
                    Err(e) => problems.push(e),
                }
            }
        }
        profiles.sort_by_key(|p| p.name.to_lowercase());
        Ok((profiles, problems))
    }

    pub fn get(&self, name: &str) -> Result<Profile> {
        let path = self.path_for(name);
        if !path.exists() {
            return Err(Error::ProfileNotFound(name.to_string()));
        }
        read_profile(&path)
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path_for(name).exists()
    }

    /// Save a profile. Checks its hotkey against other profiles and `reserved`.
    pub fn save(&self, profile: &Profile, overwrite: bool, reserved: &[(Hotkey, &str)]) -> Result<()> {
        validate_name(&profile.name)?;
        if !overwrite && self.exists(&profile.name) {
            return Err(Error::ProfileExists(profile.name.clone()));
        }
        if let Some(hk) = profile.hotkey {
            self.check_hotkey_free(hk, &profile.name, reserved)?;
        }
        let text = toml::to_string_pretty(profile).expect("profiles always serialise");
        write_atomic(&self.path_for(&profile.name), &text)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.path_for(name);
        std::fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::ProfileNotFound(name.to_string()),
            _ => Error::io(&path, e),
        })
    }

    /// Rename a profile, keeping its hotkey and monitors. Changing only the
    /// capitalisation is allowed.
    pub fn rename(&self, old: &str, new: &str) -> Result<Profile> {
        let new = new.trim();
        validate_name(new)?;
        let mut profile = self.get(old)?;
        let same_file = slug(old) == slug(new);
        if !same_file && self.exists(new) {
            return Err(Error::ProfileExists(new.to_string()));
        }
        profile.name = new.to_string();
        // Write the new file first so a failure never loses the profile.
        let text = toml::to_string_pretty(&profile).expect("profiles always serialise");
        write_atomic(&self.path_for(new), &text)?;
        if !same_file {
            self.delete(old)?;
        }
        Ok(profile)
    }

    pub fn set_hotkey(&self, name: &str, hotkey: Option<Hotkey>, reserved: &[(Hotkey, &str)]) -> Result<Profile> {
        let mut profile = self.get(name)?;
        profile.hotkey = hotkey;
        self.save(&profile, true, reserved)?;
        Ok(profile)
    }

    pub fn find_by_hotkey(&self, hotkey: Hotkey) -> Result<Option<Profile>> {
        Ok(self.list()?.into_iter().find(|p| p.hotkey == Some(hotkey)))
    }

    fn check_hotkey_free(&self, hk: Hotkey, own_name: &str, reserved: &[(Hotkey, &str)]) -> Result<()> {
        let conflict = |used_by: String| Error::HotkeyConflict {
            hotkey: hk.to_string(),
            used_by,
        };
        if let Some((_, label)) = reserved.iter().find(|(r, _)| *r == hk) {
            return Err(conflict(label.to_string()));
        }
        let own = slug(own_name);
        if let Some(other) = self
            .list()?
            .into_iter()
            .find(|p| p.hotkey == Some(hk) && slug(&p.name) != own)
        {
            return Err(conflict(format!("profile '{}'", other.name)));
        }
        Ok(())
    }
}

fn read_profile(path: &Path) -> Result<Profile> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    toml::from_str(&text).map_err(|e| Error::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample_layout;

    fn store() -> (tempfile::TempDir, ProfileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles"));
        (dir, store)
    }

    #[test]
    fn names_are_validated_and_slugged() {
        assert!(validate_name("Gaming 4K").is_ok());
        assert!(validate_name("work_setup-2").is_ok());
        for bad in [
            "",
            "   ",
            "a/b",
            "..",
            "x".repeat(65).as_str(),
            "CON",
            "nul",
            "Com1",
            "lpt9",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(slug("  Gaming   4K "), "gaming-4k");
        // Only the bare device names are reserved.
        assert!(validate_name("Console").is_ok() && validate_name("COM 1").is_ok());
        // Non-English names are fine (they're valid file names everywhere).
        assert!(validate_name("Büro").is_ok() && validate_name("ゲーム").is_ok());
    }

    #[test]
    fn save_get_list_delete() {
        let (_d, store) = store();
        assert!(store.list().unwrap().is_empty());

        let mut p = Profile::new("Gaming", sample_layout()).unwrap();
        p.hotkey = Some("Ctrl+Alt+1".parse().unwrap());
        store.save(&p, false, &[]).unwrap();
        assert_eq!(store.get("gaming").unwrap(), p);

        assert!(matches!(store.save(&p, false, &[]), Err(Error::ProfileExists(_))));
        store.save(&p, true, &[]).unwrap();

        store
            .save(&Profile::new("Desk", sample_layout()).unwrap(), false, &[])
            .unwrap();
        let names: Vec<_> = store.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Desk", "Gaming"]);

        store.delete("GAMING").unwrap();
        assert!(matches!(store.get("Gaming"), Err(Error::ProfileNotFound(_))));
        assert!(matches!(store.delete("Gaming"), Err(Error::ProfileNotFound(_))));
    }

    #[test]
    fn rename_keeps_hotkey_and_refuses_clashes() {
        let (_d, store) = store();
        let mut a = Profile::new("Desk", sample_layout()).unwrap();
        a.hotkey = Some("Ctrl+Alt+1".parse().unwrap());
        store.save(&a, false, &[]).unwrap();
        store
            .save(&Profile::new("TV", sample_layout()).unwrap(), false, &[])
            .unwrap();

        let r = store.rename("desk", "Work desk").unwrap();
        assert_eq!(r.name, "Work desk");
        assert_eq!(store.get("Work desk").unwrap().hotkey, a.hotkey);
        assert!(matches!(store.get("Desk"), Err(Error::ProfileNotFound(_))));

        store.rename("Work desk", "WORK DESK").unwrap();
        assert_eq!(store.get("work desk").unwrap().name, "WORK DESK");

        assert!(matches!(store.rename("WORK DESK", "tv"), Err(Error::ProfileExists(_))));
        assert!(matches!(
            store.rename("WORK DESK", "a/b"),
            Err(Error::InvalidProfileName(_))
        ));
        assert!(matches!(store.rename("nope", "x"), Err(Error::ProfileNotFound(_))));
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn hotkey_conflicts_are_rejected() {
        let (_d, store) = store();
        let hk: Hotkey = "Ctrl+Alt+1".parse().unwrap();
        let mut a = Profile::new("A", sample_layout()).unwrap();
        a.hotkey = Some(hk);
        store.save(&a, false, &[]).unwrap();

        // Same profile may keep its own hotkey.
        store.set_hotkey("A", Some(hk), &[]).unwrap();

        store
            .save(&Profile::new("B", sample_layout()).unwrap(), false, &[])
            .unwrap();
        let err = store.set_hotkey("B", Some(hk), &[]).unwrap_err();
        assert!(err.to_string().contains("profile 'A'"), "{err}");

        let reserved = [(Hotkey::default_duplicate(), "the duplicate-all hotkey")];
        let err = store
            .set_hotkey("B", Some(Hotkey::default_duplicate()), &reserved)
            .unwrap_err();
        assert!(err.to_string().contains("duplicate-all"), "{err}");

        assert_eq!(store.find_by_hotkey(hk).unwrap().unwrap().name, "A");
        store.set_hotkey("A", None, &[]).unwrap();
        assert!(store.find_by_hotkey(hk).unwrap().is_none());
    }

    #[test]
    fn file_format_is_readable() {
        let (_d, store) = store();
        let mut p = Profile::new("Gaming", sample_layout()).unwrap();
        p.hotkey = Some("Ctrl+Alt+1".parse().unwrap());
        store.save(&p, false, &[]).unwrap();
        let text = std::fs::read_to_string(store.dir().join("gaming.toml")).unwrap();
        assert!(text.contains("name = \"Gaming\""), "{text}");
        assert!(text.contains("hotkey = \"Ctrl+Alt+1\""), "{text}");
        assert!(text.contains("[[monitors]]"), "{text}");
    }

    #[test]
    fn corrupt_file_is_skipped_and_reported() {
        let (_d, store) = store();
        store
            .save(&Profile::new("Good", sample_layout()).unwrap(), false, &[])
            .unwrap();
        std::fs::write(store.dir().join("broken.toml"), "not = [valid").unwrap();
        std::fs::write(store.dir().join("notes.txt"), "ignored").unwrap();
        let (profiles, problems) = store.list_checked().unwrap();
        assert_eq!(profiles.len(), 1, "the good profile still loads");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().contains("broken.toml"), "{}", problems[0]);
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
