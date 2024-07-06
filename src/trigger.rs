//! Implements rising edge/falling edge/both edges trigger with hysteresis using SIMD operations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeFilter {
    Rising  = 0b01,
    Falling = 0b10,
    Both    = 0b11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Rising  = 0b01,
    Falling = 0b10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    Below,
    Above
}

#[derive(Debug, Clone, Copy)]
pub struct Trigger {
    state: State,
    level: i8, // if let Fresh = state { state = if sample < level { Below } else { Above } }
    below: i8, // if sample < below { state = Below }
    above: i8, // if sample > above { state = Above }
}

impl Trigger {
    /// Create a new trigger mechanism at `level`.
    ///
    /// The trigger mechanism detects an "above condition" when it processes a sample that is
    /// strictly above `level + hysteresis`, and a "below condition" when it processes a sample
    /// that is strictly below `level + hysteresis`. A rising edge is detected at the sample where
    /// a below condition crosses into an above condition, and a falling edge is detected at
    /// the sample where the below condition crosses into an above condition.
    ///
    /// Since `hysteresis` is applied to each half-scale individually with inequality comparisons,
    /// the total amount of hysteresis (the amount of LSBs the input value has to change by to
    /// overcome the memory of the trigger mechanism) is `1 + 2 * hysteresis`.
    ///
    /// For example, if `hysteresis` is 1 and `level` is `50`, when processing a stream of samples
    /// `[10, 49, 50, 51, 52, 53, 49, 48, 10]`, a rising edge is detected at sample #4 (value 52),
    /// and a falling edge is detected at sample #7 (value 48).
    ///
    /// The combination of level and hysteresis is clamped to the full scale such that no matter
    /// what `level` and `hysteresis` are set to, some sequence of sample values would cause
    /// a trigger to be detected.
    pub fn new(level: i8, hysteresis: u8) -> Trigger {
        Trigger {
            state: State::Fresh,
            level: level,
            below: level.saturating_sub_unsigned(hysteresis).max(-127),
            above: level.saturating_add_unsigned(hysteresis).min( 126),
        }
    }

    /// Scan incoming data for edges.
    ///
    /// The return value indicates whether processing has ended because an edge has been detected,
    /// or because no more samples could been processed. If an edge has been detected, after
    /// the function returns, `samples` point to the sample that caused the edge to be detected.
    ///
    /// This function advances `samples` forward, moving past the samples that have been processed.
    /// Trigger processing is done on groups of samples, and any samples not fitting into a group
    /// of implementation dependent size (currently 16) are left unprocessed.
    pub fn scan(&mut self, samples: &mut &[i8], filter: EdgeFilter) -> Option<Edge> {
        // Dispatch to the most efficient implementation.
        if is_x86_feature_detected!("avx2") {
            // SAFETY: The AVX2 function is called only if AVX2 is available, checked above.
            unsafe { self.scan_avx2(samples, filter) }
        } else if is_x86_feature_detected!("avx") {
            // SAFETY: The AVX function is called only if AVX is available, checked above.
            unsafe { self.scan_avx(samples, filter) }
        } else {
            self.scan_generic(samples, filter)
        }
    }

    /// Like `scan`, but returns the amount of consumed samples.
    pub fn find(&mut self, mut samples: &[i8], filter: EdgeFilter) -> (usize, Option<Edge>) {
        let len_before = samples.len();
        let edge_opt = self.scan(&mut samples, filter);
        let len_after = samples.len();
        (len_before - len_after, edge_opt)
    }
}

macro_rules! scan_impl {
    { $( $decl:tt )+ } => {
        #[inline(never)] // makes assembly more readable; serves no other purpose
        $( $decl )+(&mut self, samples: &mut &[i8], filter: EdgeFilter) -> Option<Edge> {
            // right now it is assumed that this function would be called with a holdoff of
            // the sample window size at least, i.e. that processing (00 ff)*8 with high
            // performance is not a design goal. if Nth trigger is implemented, this might have
            // to be changed.

            use wide::{i8x16, CmpGt, CmpLt};

            fn scan_for<P: Fn(i8x16) -> i8x16>(samples: &mut &[i8], predicate: P) -> bool {
                let mut found = false;
                let mut offset = 0;
                for &group in samples.array_chunks::<16>() {
                    let mask = predicate(i8x16::new(group));
                    // rustc generates ctlz even if the increment is within the condition; might
                    // as well lift it out of the condition
                    offset += (mask.move_mask() as u16).trailing_zeros() as usize;
                    if mask.any() {
                        found = true;
                        break
                    }
                }
                *samples = &samples[offset.min(samples.len())..];
                found
            }

            match (self.state, *samples) {
                (State::Fresh, []) =>
                    return None,
                (State::Fresh, [first_sample, next_samples @ ..]) => {
                    self.state = if *first_sample < self.level {
                        State::Below
                    } else {
                        State::Above
                    };
                    *samples = next_samples;
                }
                _ => ()
            }

            let above = i8x16::splat(self.above);
            let below = i8x16::splat(self.below);
            loop {
                debug_assert!(!matches!(self.state, State::Fresh));
                let found = match self.state {
                    State::Fresh => unreachable!(),
                    State::Below => scan_for(samples, |group| group.cmp_gt(above)),
                    State::Above => scan_for(samples, |group| group.cmp_lt(below)),
                };
                if found {
                    match self.state {
                        // SAFETY: `self.state == State::Fresh` is handled in the `match` above.
                        // (LLVM unconditionally elides _that_ arm and misses this one.)
                        State::Fresh => unsafe { std::hint::unreachable_unchecked() },
                        State::Below => self.state = State::Above, // rising edge
                        State::Above => self.state = State::Below, // falling edge
                    };
                    match (self.state, filter) {
                        (State::Above, EdgeFilter::Both | EdgeFilter::Rising) =>
                            return Some(Edge::Rising),
                        (State::Below, EdgeFilter::Both | EdgeFilter::Falling) =>
                            return Some(Edge::Falling),
                        _ => ()
                    }
                } else {
                    return None
                }
            }
        }
    }
}

impl Trigger {
    scan_impl! { fn scan_generic }
    scan_impl! { #[target_feature(enable = "avx")]  unsafe fn scan_avx }
    scan_impl! { #[target_feature(enable = "avx2")] unsafe fn scan_avx2 }
}

#[cfg(test)]
mod test {
    use super::*;
    use Edge::*;
    use State::*;

    macro_rules! assert_trigger {
        ($trig:ident . scan ( $data:expr , $filter:ident ) = $result:expr; +$offset:expr;
                $before:pat => $after:pat ) => {
            let mut samples = $data.as_ref();
            assert!(matches!($trig.state, $before));
            assert_eq!($trig.find(&mut samples, EdgeFilter::$filter), ($offset, $result));
            assert!(matches!($trig.state, $after));
        };
    }

    #[test]
    fn test_fresh_empty() {
        let mut trig = Trigger::new(50, 1);
        assert_trigger!(trig.scan(&[], Both) = None; +0; Fresh => Fresh);
    }

    #[test]
    fn test_fresh_above() {
        let mut trig = Trigger::new(50, 1);
        assert_trigger!(trig.scan(&[80], Both) = None; +1; Fresh => Above);
        assert_eq!(trig.above, 51);
        assert_eq!(trig.below, 49);
    }

    #[test]
    fn test_fresh_below() {
        let mut trig = Trigger::new(50, 1);
        assert_trigger!(trig.scan(&[10], Both) = None; +1; Fresh => Below);
        assert_eq!(trig.above, 51);
        assert_eq!(trig.below, 49);
    }

    fn prime_trigger(state: State) -> Trigger {
        let mut trig = Trigger::new(50, 1);
        match state {
            Fresh => {}
            Below => { trig.scan(&mut &[  0][..], EdgeFilter::Both); }
            Above => { trig.scan(&mut &[127][..], EdgeFilter::Both); }
        }
        trig
    }

    #[test]
    fn test_short() {
        let mut trig = prime_trigger(Below);
        let data = &[10, 10, 10, 10];
        assert_trigger!(trig.scan(data, Both) = None; +0; _ => Below);
    }

    const RISING_BLOCK: [i8; 16] =
        [10, 10, 10, 10, 10, 10, 10, 10, 10, 80, 80, 80, 80, 80, 80, 80];

    #[test]
    fn test_rising_both() {
        let mut trig = prime_trigger(Below);
        assert_trigger!(trig.scan(RISING_BLOCK, Both) = Some(Rising); +9; _ => Above);
    }

    #[test]
    fn test_rising_only() {
        let mut trig = prime_trigger(Below);
        assert_trigger!(trig.scan(RISING_BLOCK, Rising) = Some(Rising); +9; _ => Above);
    }

    #[test]
    fn test_rising_excluded_short() {
        let mut trig = prime_trigger(Below);
        assert_trigger!(trig.scan(RISING_BLOCK, Falling) = None; +9; _ => Above);
    }

    #[test]
    fn test_rising_excluded_long() {
        let mut trig = prime_trigger(Below);
        let data = &[
            10, 10, 10, 10, 10, 10, 10, 10, 10, 80, 80, 80, 80, 80, 80, 80,
            80, 80, 80, 80, 80, 80, 80, 80, 80,
        ];
        assert_trigger!(trig.scan(data, Falling) = None; +25; _ => Above);
    }

    #[test]
    fn test_rising_two_blocks() {
        let mut trig = prime_trigger(Below);
        let data = &[
            10, 10, 10, 10, 10, 10, 10, 10, 10, 80, 80, 80, 80, 80, 80, 80,
            80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80,
            80, 80, 80, 80, 80, 80, 80, 80, 80,
        ];
        assert_trigger!(trig.scan(data, Falling) = None; +41; _ => Above);
    }

    #[test]
    fn test_rising_almost_two_blocks() {
        let mut trig = prime_trigger(Below);
        let data = &[
            10, 10, 10, 10, 10, 10, 10, 10, 10, 80, 80, 80, 80, 80, 80, 80,
            80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80, 80,
            80, 80, 80, 80, 80, 80, 80, 80,
        ];
        assert_trigger!(trig.scan(data, Falling) = None; +25; _ => Above);
    }

    #[test]
    fn test_rising_within_dead_zone() {
        let mut trig = prime_trigger(Below);
        let data = &[
            10, 10, 10, 10, 10, 10, 10, 10, 10, 49, 49, 49, 49, 49, 49, 49,
            49, 49, 49, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10
        ];
        assert_trigger!(trig.scan(data, Falling) = None; +32; _ => Below);
    }

    const FALLING_BLOCK: [i8; 16] =
        [80, 80, 80, 80, 80, 80, 80, 80, 80, 20, 20, 20, 20, 20, 20, 20];

    #[test]
    fn test_falling_both() {
        let mut trig = prime_trigger(Above);
        assert_trigger!(trig.scan(&FALLING_BLOCK, Both) = Some(Falling); +9; _ => Below);
    }

    #[test]
    fn test_falling_only() {
        let mut trig = prime_trigger(Above);
        assert_trigger!(trig.scan(&FALLING_BLOCK, Falling) = Some(Falling); +9; _ => Below);
    }

    #[test]
    fn test_falling_excluded_short() {
        let mut trig = prime_trigger(Above);
        assert_trigger!(trig.scan(&FALLING_BLOCK, Rising) = None; +9; _ => Below);
    }

    #[test]
    fn test_falling_excluded_long() {
        let mut trig = prime_trigger(Above);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80, 20, 20, 20, 20, 20, 20, 20,
            20, 20, 20, 20, 20, 20, 20, 20, 20,
        ];
        assert_trigger!(trig.scan(data, Rising) = None; +25; _ => Below);
    }

    #[test]
    fn test_falling_two_blocks() {
        let mut trig = prime_trigger(Above);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80, 20, 20, 20, 20, 20, 20, 20,
            20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20,
            20, 20, 20, 20, 20, 20, 20, 20, 20,
        ];
        assert_trigger!(trig.scan(data, Rising) = None; +41; _ => Below);
    }

    #[test]
    fn test_falling_almost_two_blocks() {
        let mut trig = prime_trigger(Above);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80, 20, 20, 20, 20, 20, 20, 20,
            20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20,
            20, 20, 20, 20, 20, 20, 20, 20,
        ];
        assert_trigger!(trig.scan(data, Rising) = None; +25; _ => Below);
    }

    #[test]
    fn test_falling_dead_zone() { // different from test_rising_dead_zone
        let mut trig = prime_trigger(Above);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80, 50, 50, 50, 50, 50, 50, 50,
            50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
        ];
        assert_trigger!(trig.scan(data, Rising) = None; +32; _ => Above);
    }

    #[test]
    fn test_hysteresis_extreme_high() {
        let mut trig = Trigger::new(0x7f, 3);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80,  127,  127,  127,  127,  127,  127,  127,  127
        ];
        assert_trigger!(trig.scan(data, Rising) = Some(Rising); +9; _ => Above);
    }

    #[test]
    fn test_hysteresis_extreme_low() {
        let mut trig = Trigger::new(-128, 3);
        let data = &[
            80, 80, 80, 80, 80, 80, 80, 80, 80, -128, -128, -128, -128, -128, -128, -128, -128
        ];
        assert_trigger!(trig.scan(data, Falling) = Some(Falling); +9; _ => Below);
    }

    #[test]
    fn test_bug_move_mask_must_be_cast_to_u16() {
        let mut trig = prime_trigger(Below);
        let data = &[
             1,  1, -1, -3, -4, -4, -4, -5, -4, -4, -2, -2, -2, -4, -5, -5,
            -5, -5, -4, -3, -3, -3, -4, -5, -5, -5, -5, -4, -4,  0, 14, 34,
            53, 68, 77, 80, 80, 81, 83, 84, 82, 82, 82, 82, 82, 85, 88, 89,
        ];
        println!("{:?}", trig);
        assert_trigger!(trig.scan(data, Rising) = Some(Rising); +32; _ => Above);
    }
}
