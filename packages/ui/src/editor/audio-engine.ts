/**
 * Simple Web Audio API oscillator for buzzer simulation.
 * Plays a tone at a given frequency when active.
 */

let audioCtx: AudioContext | null = null;
let oscillator: OscillatorNode | null = null;
let gainNode: GainNode | null = null;

function getContext(): AudioContext {
  if (!audioCtx) {
    audioCtx = new AudioContext();
  }
  return audioCtx;
}

/**
 * Start playing a tone at the given frequency.
 * If already playing, updates the frequency.
 */
export function startTone(frequency: number, volume = 0.15): void {
  const ctx = getContext();

  if (!oscillator) {
    oscillator = ctx.createOscillator();
    gainNode = ctx.createGain();
    oscillator.type = 'square'; // Buzzer-like sound
    oscillator.connect(gainNode);
    gainNode.connect(ctx.destination);
    oscillator.start();
  }

  oscillator.frequency.setValueAtTime(frequency, ctx.currentTime);
  gainNode!.gain.setValueAtTime(volume, ctx.currentTime);
}

/**
 * Stop the currently playing tone.
 */
export function stopTone(): void {
  if (oscillator) {
    oscillator.stop();
    oscillator.disconnect();
    oscillator = null;
  }
  if (gainNode) {
    gainNode.disconnect();
    gainNode = null;
  }
}

/**
 * Resume audio context (must be called from a user gesture).
 */
export function resumeAudio(): void {
  if (audioCtx?.state === 'suspended') {
    audioCtx.resume();
  }
}
