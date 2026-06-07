export type BoardIoKind =
  | 'led' | 'button' | 'adc_input' | 'pwm_output' | 'i2c_device' | 'spi_device';
export interface Part { id: string; type: string; attrs?: Record<string, string>; }
export interface WireEndpoint { part: string; pin: string; }
export interface Wire { from: WireEndpoint; to: WireEndpoint; }
export interface Diagram { board: string; parts: Part[]; wires: Wire[]; }
