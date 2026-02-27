use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmSimulator {}

#[wasm_bindgen]
impl WasmSimulator {
    #[wasm_bindgen(constructor)]
    pub fn new(_firmware: &[u8]) -> Result<WasmSimulator, JsValue> {
        Err(JsValue::from_str("Not implemented due to core refactor"))
    }

    #[wasm_bindgen]
    pub fn step(&mut self, _cycles: u32) -> Result<(), JsValue> {
        Err(JsValue::from_str("Not implemented"))
    }

    #[wasm_bindgen]
    pub fn get_pc(&self) -> u32 {
        0
    }

    #[wasm_bindgen]
    pub fn get_led_state(&mut self) -> bool {
        false
    }
}
