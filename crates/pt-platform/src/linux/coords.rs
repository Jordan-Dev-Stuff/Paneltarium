//! Moving an arrangement between two coordinate spaces where each monitor
//! has a different size.
//!
//! Paneltarium's profiles (like Windows) place monitors in *pixels*: a 4K
//! monitor is 3840 wide. Wayland compositors place them in *logical*
//! (scaled) units: the same 4K monitor at 200 % is 1920 wide. With mixed
//! scales there is no single factor between the two, so instead the
//! arrangement is rebuilt monitor by monitor: whatever touched what keeps
//! touching it, on the same side, with edges that were lined up staying
//! lined up.

/// A monitor's rectangle in the source space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Rect {
            x: x.into(),
            y: y.into(),
            w: w.into(),
            h: h.into(),
        }
    }

    fn x1(&self) -> i64 {
        self.x + self.w
    }

    fn y1(&self) -> i64 {
        self.y + self.h
    }
}

/// Positions in the target space for monitors at `from` (source space)
/// whose sizes in the target space are `to_sizes`. Monitor `root` lands at
/// (0, 0); the others are placed next to the neighbours they touched.
pub fn convert(from: &[Rect], to_sizes: &[(u32, u32)], root: usize) -> Vec<(i32, i32)> {
    assert_eq!(from.len(), to_sizes.len());
    let n = from.len();
    let size = |i: usize| (i64::from(to_sizes[i].0), i64::from(to_sizes[i].1));
    let mut placed: Vec<Option<(i64, i64)>> = vec![None; n];
    if n == 0 {
        return Vec::new();
    }
    placed[root] = Some((0, 0));
    loop {
        let mut progress = false;
        for m in 0..n {
            if placed[m].is_some() {
                continue;
            }
            placed[m] = (0..n).find_map(|k| place(from[m], size(m), from[k], placed[k]?, size(k)));
            progress |= placed[m].is_some();
        }
        if !progress {
            break;
        }
    }
    // Monitors touching nothing (not a valid desktop, but don't lose them):
    // scale their offset from the root by the root's own factor.
    let r = from[root];
    let (rw, rh) = size(root);
    placed
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = p.unwrap_or_else(|| {
                (
                    div_round((from[i].x - r.x) * rw, r.w.max(1)),
                    div_round((from[i].y - r.y) * rh, r.h.max(1)),
                )
            });
            (clamp(x), clamp(y))
        })
        .collect()
}

/// Where monitor `m` goes if it touches the already placed monitor `n`.
fn place(m: Rect, m_size: (i64, i64), n: Rect, n_pos: (i64, i64), n_size: (i64, i64)) -> Option<(i64, i64)> {
    if m == n {
        return Some(n_pos); // duplicated: same picture, same place
    }
    let overlaps = |a0: i64, a1: i64, b0: i64, b1: i64| a0 < b1 && b0 < a1;
    let along_y = || n_pos.1 + offset(m.y, m.y1(), n.y, n.y1(), n_size.1, m_size.1);
    let along_x = || n_pos.0 + offset(m.x, m.x1(), n.x, n.x1(), n_size.0, m_size.0);
    if overlaps(m.y, m.y1(), n.y, n.y1()) {
        if m.x == n.x1() {
            return Some((n_pos.0 + n_size.0, along_y()));
        }
        if m.x1() == n.x {
            return Some((n_pos.0 - m_size.0, along_y()));
        }
    }
    if overlaps(m.x, m.x1(), n.x, n.x1()) {
        if m.y == n.y1() {
            return Some((along_x(), n_pos.1 + n_size.1));
        }
        if m.y1() == n.y {
            return Some((along_x(), n_pos.1 - m_size.1));
        }
    }
    None
}

/// Offset of `m` along the edge it shares with `n`, in the target space.
/// Lined-up starts or ends stay lined up; anything else is scaled by `n`'s
/// factor along that edge.
fn offset(m0: i64, m1: i64, n0: i64, n1: i64, n_len: i64, m_len: i64) -> i64 {
    if m0 == n0 {
        0
    } else if m1 == n1 {
        n_len - m_len
    } else {
        div_round((m0 - n0) * n_len, (n1 - n0).max(1))
    }
}

fn div_round(a: i64, b: i64) -> i64 {
    (a as f64 / b as f64).round() as i64
}

fn clamp(v: i64) -> i32 {
    v.clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn same_sizes_keep_the_arrangement() {
        let from = [
            r(0, 0, 1920, 1080),
            r(1920, 200, 2560, 1440),
            r(-1080, -300, 1080, 1920),
        ];
        let sizes = [(1920, 1080), (2560, 1440), (1080, 1920)];
        assert_eq!(convert(&from, &sizes, 0), [(0, 0), (1920, 200), (-1080, -300)]);
    }

    #[test]
    fn hidpi_monitor_beside_a_normal_one() {
        // Logical: 4K at 200 % (1920 wide) at 0, a 1080p at 1920.
        let logical = [r(0, 0, 1920, 1080), r(1920, 0, 1920, 1080)];
        let physical = [(3840, 2160), (1920, 1080)];
        assert_eq!(convert(&logical, &physical, 0), [(0, 0), (3840, 0)]);
        // And back again.
        let px = [r(0, 0, 3840, 2160), r(3840, 0, 1920, 1080)];
        assert_eq!(convert(&px, &[(1920, 1080), (1920, 1080)], 0), [(0, 0), (1920, 0)]);
    }

    #[test]
    fn bottom_aligned_stays_bottom_aligned() {
        // Laptop (1080p, 100 %) left of a 4K at 150 % (2560x1440 logical),
        // bottoms lined up.
        let logical = [r(0, 360, 1920, 1080), r(1920, 0, 2560, 1440)];
        let physical = [(1920, 1080), (3840, 2160)];
        assert_eq!(convert(&logical, &physical, 1), [(-1920, 1080), (0, 0)]);
    }

    #[test]
    fn stacked_and_chained() {
        // TV above the desk monitor, a third monitor right of the desk one,
        // everything at 200 % in logical space.
        let logical = [r(0, 0, 960, 540), r(0, -1080, 1920, 1080), r(960, 0, 960, 540)];
        let physical = [(1920, 1080), (3840, 2160), (1920, 1080)];
        assert_eq!(convert(&logical, &physical, 0), [(0, 0), (0, -2160), (1920, 0)]);
    }

    #[test]
    fn partial_offsets_scale_with_the_neighbour() {
        // A monitor hanging halfway down the right edge of a 200 % one.
        let logical = [r(0, 0, 1920, 1080), r(1920, 540, 1920, 1080)];
        let physical = [(3840, 2160), (1920, 1080)];
        assert_eq!(convert(&logical, &physical, 0), [(0, 0), (3840, 1080)]);
    }

    #[test]
    fn duplicates_share_a_position() {
        let from = [r(0, 0, 1920, 1080), r(0, 0, 1920, 1080)];
        assert_eq!(convert(&from, &[(3840, 2160), (3840, 2160)], 0), [(0, 0), (0, 0)]);
    }

    #[test]
    fn islands_are_scaled_from_the_root() {
        let from = [r(0, 0, 1920, 1080), r(4000, 0, 1920, 1080)];
        assert_eq!(convert(&from, &[(3840, 2160), (1920, 1080)], 0), [(0, 0), (8000, 0)]);
        assert!(convert(&[], &[], 0).is_empty());
    }
}
