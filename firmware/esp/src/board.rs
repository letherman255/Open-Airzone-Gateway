//! Board pin maps, chosen at build time with `AZ_BOARD` (see build.rs):
//! - `waveshare-esp32s3-rs485` (default): RX 18, TX 17, DE 21, BOOT button 0;
//! - `generic-esp32-max485`: pins from `AZ_PIN_RX`, `AZ_PIN_TX`, `AZ_PIN_DE`, optional `AZ_PIN_BUTTON`.
//! Add a board here and in build.rs, and document its wiring in hardware/.

#[derive(Debug, Clone, Copy)]
pub struct Board {
    pub name: &'static str,
    /// UART RX from the RS-485 transceiver.
    pub uart_rx: i32,
    /// UART TX to the transceiver (master builds only).
    #[allow(dead_code)]
    pub uart_tx: i32,
    /// RS-485 driver enable (high = transmit). Held low unless the master engine runs.
    pub de: i32,
    /// Button held 5 s = network reset (active low).
    pub button: Option<i32>,
}

include!(concat!(env!("OUT_DIR"), "/board_gen.rs"));
