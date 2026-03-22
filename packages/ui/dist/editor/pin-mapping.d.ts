/**
 * Maps MCU pin IDs to their alternate functions (ADC channels, I2C buses, SPI buses, timers, etc.)
 * Used by diagramToConfig to auto-detect connection types from wires.
 */
export interface PinFunction {
    type: 'gpio' | 'adc' | 'i2c' | 'spi' | 'timer' | 'uart';
    peripheral: string;
    channel?: number;
    role?: string;
}
export interface PinMapping {
    gpio: {
        peripheral: string;
        pin: number;
    };
    functions: PinFunction[];
}
/**
 * Look up a pin's mapping for a given board.
 */
export declare function getPinMapping(board: string, pinLabel: string): PinMapping | null;
/**
 * Find a specific alternate function for a pin.
 */
export declare function findPinFunction(board: string, pinLabel: string, type: PinFunction['type']): PinFunction | null;
