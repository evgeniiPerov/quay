//! Braille-dot spinner widget driven by the application tick clock.

/// A ten-frame braille spinner that advances one step per tick.
///
/// Call [`Spinner::advance`] from the screen's tick handler (once per tick)
/// and render the current frame with [`Spinner::frame`].
#[derive(Debug, Default, Clone, Copy)]
pub struct Spinner {
    tick: u64,
}

impl Spinner {
    /// Advance the spinner by one frame.
    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Return the glyph for the current frame.
    pub fn frame(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[(self.tick as usize) % FRAMES.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_ten_frames() {
        let mut s = Spinner::default();
        let first = s.frame();
        for _ in 0..10 {
            s.advance();
        }
        // After exactly 10 advances the frame wraps back to the start.
        assert_eq!(s.frame(), first);
    }
}
