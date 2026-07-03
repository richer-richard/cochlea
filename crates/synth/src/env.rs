//! Piecewise-linear ADSR as a pure function of time. No exponential
//! segments in v1: linear ramps are pure arithmetic, so the envelope is
//! bit-deterministic on every platform. The release always ramps from the
//! envelope's value *at note-off*, so a note shorter than attack+decay
//! releases without a jump.

/// ADSR times in seconds, sustain as a level in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adsr {
    pub attack: f64,
    pub decay: f64,
    pub sustain: f64,
    pub release: f64,
}

impl Adsr {
    /// The pre-release envelope: attack ramp, decay ramp, sustain plateau.
    fn held(self, t: f64) -> f64 {
        if t < self.attack {
            t / self.attack
        } else if t < self.attack + self.decay {
            1.0 - (1.0 - self.sustain) * (t - self.attack) / self.decay
        } else {
            self.sustain
        }
    }

    /// Envelope value at `t` seconds after note-on, for a note held
    /// `note_len` seconds. Continuous everywhere; zero after release ends.
    pub fn value(self, t: f64, note_len: f64) -> f64 {
        if t < note_len {
            self.held(t)
        } else if t - note_len >= self.release {
            0.0 // exactly zero once released, no residual ulps
        } else {
            let level = self.held(note_len);
            let ramp = 1.0 - (t - note_len) / self.release;
            (level * ramp).max(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Adsr = Adsr {
        attack: 0.01,
        decay: 0.1,
        sustain: 0.6,
        release: 0.2,
    };

    #[test]
    fn adsr_traces_its_segments() {
        assert_eq!(A.value(0.0, 1.0), 0.0);
        assert_eq!(A.value(0.01, 1.0), 1.0);
        assert!((A.value(0.06, 1.0) - 0.8).abs() < 1e-12); // mid-decay
        assert_eq!(A.value(0.5, 1.0), 0.6); // sustain
        assert!((A.value(1.1, 1.0) - 0.3).abs() < 1e-12); // mid-release
        // 1.2 - 1.0 lands one ulp inside the release in f64; the residual
        // is sub-1e-15 and the envelope pins to exactly 0 past the end.
        assert!(A.value(1.2, 1.0).abs() < 1e-12);
        assert_eq!(A.value(1.2000001, 1.0), 0.0);
        assert_eq!(A.value(5.0, 1.0), 0.0);
    }

    #[test]
    fn a_note_released_mid_attack_ramps_from_its_current_level() {
        // note_len 5 ms: release starts at attack level 0.5, no jump.
        let at_release = A.value(0.005, 0.005);
        assert!((at_release - 0.5).abs() < 1e-12);
        let just_after = A.value(0.0051, 0.005);
        assert!((at_release - just_after).abs() < 0.01);
    }
}
