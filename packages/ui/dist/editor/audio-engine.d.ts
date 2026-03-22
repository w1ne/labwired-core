/**
 * Simple Web Audio API oscillator for buzzer simulation.
 * Plays a tone at a given frequency when active.
 */
/**
 * Start playing a tone at the given frequency.
 * If already playing, updates the frequency.
 */
export declare function startTone(frequency: number, volume?: number): void;
/**
 * Stop the currently playing tone.
 */
export declare function stopTone(): void;
/**
 * Resume audio context (must be called from a user gesture).
 */
export declare function resumeAudio(): void;
