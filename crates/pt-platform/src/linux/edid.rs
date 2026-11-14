//! Reading a monitor's EDID: the manufacturer, product code, serial number
//! and name it reports about itself. Every display stack exposes the
//! connector name (`DP-1`, `HDMI-A-1`, ...); the kernel publishes the raw EDID
//! for each connector under `/sys/class/drm/card*-<connector>/edid`, so the
//! same stable id works with GNOME, KDE, wlroots and X11.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// Three-letter PNP id packed as in the EDID (bytes 8–9, big-endian).
    pub manufacturer: u16,
    /// Product code (bytes 10–11, little-endian).
    pub product: u16,
    /// Serial number: the text descriptor if there is one, else the
    /// numeric field (bytes 12–15) in hex.
    pub serial: String,
    /// Monitor name descriptor (0xFC), e.g. "DELL U2720Q".
    pub name: Option<String>,
}

impl Edid {
    pub fn parse(bytes: &[u8]) -> Option<Edid> {
        const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
        if bytes.len() < 128 || bytes[..8] != HEADER {
            return None;
        }
        let mut name = None;
        let mut serial_text = None;
        // Four 18-byte descriptors; display descriptors start with 00 00 00.
        for d in bytes[54..126].chunks_exact(18) {
            if d[..3] != [0, 0, 0] {
                continue;
            }
            let text = || {
                let raw = &d[5..18];
                let end = raw.iter().position(|&b| b == 0x0A).unwrap_or(raw.len());
                let s: String = raw[..end].iter().map(|&b| b as char).collect();
                Some(s.trim().to_string()).filter(|s| !s.is_empty())
            };
            match d[3] {
                0xFC => name = text(),
                0xFF => serial_text = text(),
                _ => {}
            }
        }
        let number = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        Some(Edid {
            manufacturer: u16::from_be_bytes([bytes[8], bytes[9]]),
            product: u16::from_le_bytes([bytes[10], bytes[11]]),
            serial: serial_text.unwrap_or_else(|| format!("{number:08X}")),
            name,
        })
    }

    /// The manufacturer as its three letters, e.g. "DEL".
    pub fn vendor(&self) -> String {
        pnp_letters(self.manufacturer)
    }

    /// Parse the hex dump `xrandr --verbose` prints (whitespace ignored).
    pub fn from_hex(hex: &str) -> Option<Edid> {
        let digits: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        if !digits.len().is_multiple_of(2) {
            return None;
        }
        let bytes: Option<Vec<u8>> = digits
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
            .collect();
        Edid::parse(&bytes?)
    }
}

/// "DEL" → the packed 16-bit PNP id (5 bits per letter, 'A' = 1).
pub fn pnp_id(letters: &str) -> Option<u16> {
    let b = letters.as_bytes();
    if b.len() != 3 || !b.iter().all(u8::is_ascii_uppercase) {
        return None;
    }
    Some(b.iter().fold(0u16, |acc, &c| (acc << 5) | u16::from(c - b'@')))
}

pub fn pnp_letters(id: u16) -> String {
    [10, 5, 0]
        .iter()
        .map(|shift| (b'@' + ((id >> shift) & 0x1F) as u8) as char)
        .collect()
}

/// The EDID of `connector` (e.g. `DP-1`), from the kernel.
pub fn for_connector(connector: &str) -> Option<Edid> {
    for_connector_in(Path::new("/sys/class/drm"), connector)
}

fn for_connector_in(drm: &Path, connector: &str) -> Option<Edid> {
    let suffix = format!("-{connector}");
    let mut entries: Vec<_> = std::fs::read_dir(drm)
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("card") && name.ends_with(&suffix)
        })
        .collect();
    // Stable choice if several GPUs have a connector of that name.
    entries.sort_by_key(|e| e.file_name());
    entries
        .iter()
        .find_map(|e| Edid::parse(&std::fs::read(e.path().join("edid")).ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal valid EDID: manufacturer "DEL", product 0xA0B1, serial
    /// 0x12345678, with a name and (optionally) a serial-number descriptor.
    pub fn sample(name: &str, serial_text: Option<&str>) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        e[8..10].copy_from_slice(&pnp_id("DEL").unwrap().to_be_bytes());
        e[10..12].copy_from_slice(&0xA0B1u16.to_le_bytes());
        e[12..16].copy_from_slice(&0x12345678u32.to_le_bytes());
        let mut descriptor = |slot: usize, tag: u8, text: &str| {
            let d = &mut e[54 + slot * 18..54 + (slot + 1) * 18];
            d[3] = tag;
            let mut t = text.as_bytes().to_vec();
            t.push(0x0A);
            t.resize(13, b' ');
            d[5..18].copy_from_slice(&t);
        };
        descriptor(1, 0xFC, name);
        if let Some(s) = serial_text {
            descriptor(2, 0xFF, s);
        }
        e
    }

    #[test]
    fn parses_ids_name_and_serial() {
        let e = Edid::parse(&sample("DELL U2720Q", None)).unwrap();
        assert_eq!(e.vendor(), "DEL");
        assert_eq!(e.product, 0xA0B1);
        assert_eq!(e.serial, "12345678");
        assert_eq!(e.name.as_deref(), Some("DELL U2720Q"));

        let e = Edid::parse(&sample("LG TV", Some("ABC123"))).unwrap();
        assert_eq!(e.serial, "ABC123", "the text serial wins");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Edid::parse(&[0u8; 128]).is_none());
        assert!(Edid::parse(&sample("X", None)[..100]).is_none());
        assert!(Edid::from_hex("00ff").is_none());
        assert!(Edid::from_hex("zz").is_none());
    }

    #[test]
    fn hex_dump_round_trips() {
        let bytes = sample("DELL U2720Q", None);
        let hex: String = bytes
            .chunks(16)
            .map(|row| format!("\t\t{}\n", row.iter().map(|b| format!("{b:02x}")).collect::<String>()))
            .collect();
        assert_eq!(Edid::from_hex(&hex), Edid::parse(&bytes));
    }

    #[test]
    fn pnp_ids_round_trip() {
        for s in ["DEL", "GSM", "BOE", "AAA", "ZZZ"] {
            assert_eq!(pnp_letters(pnp_id(s).unwrap()), s);
        }
        assert_eq!(pnp_id("DEL"), Some(0x10AC));
        assert_eq!(pnp_id("del"), None);
        assert_eq!(pnp_id("DELL"), None);
    }

    #[test]
    fn finds_the_connector_in_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        for (entry, name) in [
            ("card1-DP-1", "Left"),
            ("card1-DP-11", "Wrong"),
            ("card1-HDMI-A-1", "TV"),
        ] {
            std::fs::create_dir_all(dir.path().join(entry)).unwrap();
            std::fs::write(dir.path().join(entry).join("edid"), sample(name, None)).unwrap();
        }
        // An unplugged connector has an empty edid file.
        std::fs::create_dir_all(dir.path().join("card1-DP-2")).unwrap();
        std::fs::write(dir.path().join("card1-DP-2/edid"), b"").unwrap();

        let name = |c| for_connector_in(dir.path(), c).and_then(|e| e.name);
        assert_eq!(name("DP-1").as_deref(), Some("Left"));
        assert_eq!(name("HDMI-A-1").as_deref(), Some("TV"));
        assert_eq!(name("DP-2"), None);
        assert_eq!(name("DP-3"), None);
    }
}
