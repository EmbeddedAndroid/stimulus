#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub page0: u16,
    pub n: u16,
}

pub fn plan_window(wr: u16, post_plus_one: u16, compressed: bool) -> Window {
    let wr = wr.min(2047);
    let post = if compressed {
        0
    } else {
        post_plus_one.min(2047)
    };
    // Window::n is the number of pages to fetch, while the vendor's local N is
    // the inclusive last index (n - 1).
    let n = 2048 - post;
    let page0 = (u32::from(wr) + 1 + u32::from(post)) as u16 % 2048;
    Window { page0, n }
}

pub const fn adjust_for_run_probe(mut window: Window, flags: u8) -> Window {
    if flags & 8 != 0 && window.n > 0 {
        window.page0 = (window.page0 + 1) % 2048;
        window.n -= 1;
    }
    window
}

pub fn trigger_index(n: u16, trig_page: u16) -> usize {
    let span = usize::from(n) + 1;
    (usize::from(n) + span - usize::from(trig_page.min(n)) + 1) % span
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocks {
    pub b: [Vec<u8>; 4],
    pub flags: Vec<u8>,
    pub ddr: Option<[Vec<u8>; 4]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_window_reference_examples() {
        let vectors = [
            (
                0x0e7,
                0x7f3,
                false,
                0,
                Window {
                    page0: 0x0db,
                    n: 13,
                },
            ),
            (
                0x24f,
                0x7f3,
                false,
                0,
                Window {
                    page0: 0x243,
                    n: 13,
                },
            ),
            (
                0x169,
                0x7a5,
                false,
                0,
                Window {
                    page0: 0x10f,
                    n: 91,
                },
            ),
            (
                0x5bf,
                0x4c9,
                false,
                0,
                Window {
                    page0: 0x289,
                    n: 823,
                },
            ),
            (
                0x56d,
                0,
                true,
                8,
                Window {
                    page0: 0x56f,
                    n: 2047,
                },
            ),
        ];
        for (wr, post, compressed, probe, expected) in vectors {
            assert_eq!(
                adjust_for_run_probe(plan_window(wr, post, compressed), probe),
                expected
            );
        }
    }

    #[test]
    fn trigger_index_wraps_ring() {
        assert_eq!(trigger_index(2047, 0), 0);
        assert_eq!(trigger_index(13, 13), 1);
    }
}
