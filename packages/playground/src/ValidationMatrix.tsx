// Public Tier-1 validation matrix — the "public trace". One source of truth:
// docs/coverage/tier1-matrix.json on core main (nightly-refreshed with CI
// run_url evidence). Proof-artifact bar: a cell renders its status ONLY if it
// carries a run_url; status without evidence renders as unrecorded.
import { useEffect, useRef, useState } from 'react';

export const MATRIX_URL =
  'https://raw.githubusercontent.com/w1ne/labwired-core/main/docs/coverage/tier1-matrix.json';

/// Dev/preview override: ?matrix=<url> swaps the data source (e.g. a local
/// sample under /public) without touching the production constant.
function matrixUrl(): string {
  if (typeof window === 'undefined') return MATRIX_URL;
  return new URLSearchParams(window.location.search).get('matrix') ?? MATRIX_URL;
}

const RUBRIC = [
  // bring-up rubric
  'clock', 'gpio', 'uart', 'timer', 'dma', 'irq',
  // typical MCU peripherals (standard columns as of the 15-chip matrix)
  'i2c', 'spi', 'adc', 'pwm', 'wdt', 'rtc',
];

type Cell = { status: string; run_url?: string };

// Proper part names for display — row keys stay the stable chip ids.
const DISPLAY_NAMES: Record<string, string> = {
  esp32: 'ESP32 (Xtensa LX6)',
  esp32c3: 'ESP32-C3 (RISC-V)',
  esp32s3: 'ESP32-S3 (Xtensa LX7)',
  nrf52832: 'nRF52832',
  nrf52840: 'nRF52840',
  rp2040: 'RP2040',
  stm32f103: 'STM32F103C8',
  stm32f401: 'STM32F401RE',
  stm32f407: 'STM32F407VG',
  stm32g474re: 'STM32G474RE',
  stm32h563: 'STM32H563',
  stm32l073: 'STM32L073RZ',
  stm32l476: 'STM32L476RG',
  stm32wb55: 'STM32WB55',
  stm32wba52: 'STM32WBA52',
};
type Matrix = Record<string, Record<string, Cell>>;

const ICON: Record<string, string> = {
  pass: '✅',
  partial: '🟡',
  blocked: '⛔',
  na: '🚧',
  unrecorded: '·',
};

// Unknown status (schema drift): render '?' so drift is visible.
function iconFor(status: string): string {
  return ICON[status] ?? '?';
}

function effectiveStatus(cell: Cell | undefined): { status: string; url?: string } {
  if (!cell) return { status: 'unrecorded' };
  if (cell.status === 'na' || cell.status === 'unrecorded') return { status: cell.status };
  if (!cell.run_url) return { status: 'unrecorded' }; // no evidence, no claim
  return { status: cell.status, url: cell.run_url };
}

export function ValidationMatrix() {
  const [matrix, setMatrix] = useState<Matrix | null>(null);
  const [error, setError] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    abortRef.current = controller;
    fetch(matrixUrl(), { signal: controller.signal })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(setMatrix)
      .catch((err: unknown) => {
        if (err instanceof Error && err.name === 'AbortError') return;
        setError(true);
      });
    return () => controller.abort();
  }, []);

  if (error) {
    return (
      <p className="validation-empty text-fg-secondary text-[15px] py-10 text-center">
        Validation data unavailable.
      </p>
    );
  }
  if (!matrix) {
    return (
      <p className="validation-empty text-fg-secondary text-[15px] py-10 text-center">
        Loading validation matrix…
      </p>
    );
  }

  const chips = Object.keys(matrix).sort();

  if (chips.length === 0) {
    return (
      <p className="validation-empty text-fg-secondary text-[15px] py-10 text-center">
        Validation data unavailable.
      </p>
    );
  }

  // Overview = the 12 universal subsystems only. Chip-specific peripherals
  // (e.g. ESP32 RMT) are intentionally excluded here — they exist on some
  // parts but not others, so they can't share an apples-to-apples grid where
  // 🚧 means "not modeled yet". They belong in the per-chip detail report.
  const classes = RUBRIC;

  return (
    <section className="validation-matrix">
      <h2 className="text-[32px] md:text-[40px] font-bold tracking-tight mb-3 text-fg-primary">
        What Tier-1 means
      </h2>
      <p className="text-fg-secondary text-[16px] leading-[1.6] mb-10 max-w-[70ch]">
        Tier-1 is what every supported chip must prove on real firmware: the
        bring-up six (clock, GPIO, UART, timers, DMA, interrupt routing) plus the
        typical peripherals — I²C, SPI, ADC, PWM, watchdog, RTC. A green cell is
        sim-consistent: the check passed against the simulator&rsquo;s peripheral
        models. Silicon-anchored verification is a separate tier, coming with
        hardware-in-the-loop. Because every chip here has all of these
        subsystems in silicon, a non-green cell is honest about why: ⛔ is a
        modeled subsystem that&rsquo;s currently failing, 🚧 is a subsystem the
        model doesn&rsquo;t cover yet, and a dot is a check we haven&rsquo;t
        written.
      </p>

      {/* Legend */}
      <div className="flex flex-wrap gap-x-6 gap-y-2 mb-8 text-[13px] text-fg-secondary font-mono">
        {([
          ['pass', '✅', 'passed with evidence'],
          ['partial', '🟡', 'partial coverage'],
          ['blocked', '⛔', 'modeled, failing'],
          ['unrecorded', '·', 'no check written'],
          ['na', '🚧', 'not modeled yet'],
        ] as const).map(([, icon, desc]) => (
          <span key={desc} className="flex items-center gap-1.5">
            <span className="text-[15px]">{icon}</span>
            <span className="text-fg-tertiary">{desc}</span>
          </span>
        ))}
      </div>

      {/* House card wrapper — mirrors CiLanding.tsx:247 */}
      <div className="overflow-x-auto border-2 border-[#1a1a1a] rounded-[10px] shadow-[5px_5px_0_#1a1a1a] overflow-hidden bg-white">
        <table className="w-full border-collapse text-[13px]">
          <thead>
            <tr className="border-b-2 border-[#1a1a1a] bg-[#f8f9fa]">
              <th className="text-left py-3 px-4 text-fg-tertiary font-bold uppercase tracking-wider text-[11px] whitespace-nowrap">
                chip
              </th>
              {classes.map((c) => (
                <th
                  key={c}
                  className={`py-3 px-4 text-fg-tertiary font-bold uppercase tracking-wider text-[11px] whitespace-nowrap text-center`}
                >
                  {c}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {chips.map((chip, chipIdx) => (
              <tr
                key={chip}
                className={chipIdx < chips.length - 1 ? 'border-b border-[#d6d8dc]' : ''}
              >
                <td className="py-3 px-4 font-mono text-fg-primary font-semibold whitespace-nowrap text-[12.5px]">
                  {DISPLAY_NAMES[chip] ?? chip}
                </td>
                {classes.map((cls) => {
                  const { status, url } = effectiveStatus(matrix[chip]?.[cls]);
                  const label = `${cls}: ${status}`;
                  return (
                    <td key={cls} className="py-3 px-4 text-center">
                      {url ? (
                        <a
                          href={url}
                          aria-label={label}
                          target="_blank"
                          rel="noreferrer"
                          className="inline-flex items-center justify-center w-8 h-8 rounded-[6px] transition-all duration-150 hover:bg-[#f0f4ff] hover:scale-110 hover:shadow-[2px_2px_0_#0056b3] text-[17px]"
                          title={`${chip} / ${cls}: ${status} — view CI run`}
                        >
                          {iconFor(status)}
                        </a>
                      ) : (
                        <span
                          role="img"
                          aria-label={label}
                          className="inline-flex items-center justify-center w-8 h-8 rounded-[6px] text-fg-tertiary text-[17px]"
                          title={`${chip} / ${cls}: ${status}`}
                        >
                          {iconFor(status)}
                        </span>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <p className="text-fg-tertiary text-[12px] mt-4">
        Source:{' '}
        <a
          href="https://github.com/w1ne/labwired-core/blob/main/docs/coverage/tier1-matrix.json"
          target="_blank"
          rel="noreferrer"
          className="text-accent hover:underline font-medium"
        >
          docs/coverage/tier1-matrix.json
        </a>{' '}
        on core main — refreshed nightly by CI.
      </p>
    </section>
  );
}
