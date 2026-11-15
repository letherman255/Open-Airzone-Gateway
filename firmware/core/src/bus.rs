//! The physical bus as the master engine sees it. Plan 2 implements it on the ESP32 UART; the simulator
//! implements it for tests.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusError {
    Timeout,
    Io,
}

pub trait Bus {
    /// Send one request frame and return the reply frame (CRC included). `Timeout` when nothing answers.
    fn transact(&mut self, request: &[u8], timeout_ms: u32) -> Result<Vec<u8>, BusError>;
    /// Receive only, for `duration_ms`; returns the frames heard (used to detect another master).
    fn listen(&mut self, duration_ms: u32) -> Vec<Vec<u8>>;
}
