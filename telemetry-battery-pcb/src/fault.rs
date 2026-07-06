//! Edge-triggered latch used to avoid spamming the CAN bus with the same warning/error every time
//! a condition is (still) true: [`Latch::rising_edge`] only returns `true` the first time the
//! condition becomes true, and resets once the condition clears.

pub struct Latch {
    active: bool,
}

impl Latch {
    pub const fn new() -> Self {
        Self { active: false }
    }

    /// Update the latch with the current state of the monitored condition. Returns `true` exactly
    /// when `condition` transitions from `false` to `true`.
    pub fn rising_edge(&mut self, condition: bool) -> bool {
        let triggered = condition && !self.active;
        self.active = condition;

        triggered
    }
}
