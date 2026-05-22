import type { CompileError } from './CodeEditor';

export interface CompileResult {
  success: boolean;
  elf?: Uint8Array;
  errors: CompileError[];
  output: string;
}

export interface CompileOptions {
  source: string;
  language: 'c' | 'cpp' | 'arduino';
  target: string; // e.g. 'stm32f103', 'stm32f401'
}

/**
 * Compile source code to an ELF binary.
 *
 * This currently uses a mock implementation that returns pre-built firmware.
 * When a compile server is available, it will POST to /api/compile.
 */
export async function compileCode(options: CompileOptions): Promise<CompileResult> {
  const { source } = options;

  // Basic syntax validation (client-side)
  const errors = validateSyntax(source);
  if (errors.length > 0) {
    return { success: false, errors, output: 'Compilation failed with syntax errors.' };
  }

  // Try compile servers. In dev, try localhost:3001 first (typical local
  // compile-server port). Same-origin /api/compile is the prod hook — only
  // attempted when an explicit env override is set, because app.labwired.com
  // doesn't ship one and the request just 405s noisily otherwise.
  const isDev = typeof import.meta !== 'undefined' && (import.meta as { env?: { DEV?: boolean } }).env?.DEV;
  const override = typeof import.meta !== 'undefined' && (import.meta as { env?: { VITE_COMPILE_URL?: string } }).env?.VITE_COMPILE_URL;
  const urls: string[] = [];
  if (override) urls.push(override);
  if (isDev) urls.push('http://localhost:3001/api/compile');
  for (const url of urls) {
  try {
    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(options),
    });

    if (resp.ok) {
      const data = await resp.json();
      if (data.success) {
        const elfData = Uint8Array.from(atob(data.elf), (c) => c.charCodeAt(0));
        return { success: true, elf: elfData, errors: [], output: data.output ?? 'Compilation successful.' };
      }
      return {
        success: false,
        errors: data.errors ?? [],
        output: data.output ?? 'Compilation failed.',
      };
    }
  } catch {
    // This server not available — try next
    continue;
  }
  } // end for

  // Demo mode: return a "no compile server" message
  return {
    success: false,
    errors: [],
    output: 'No compile server available. Use a pre-built demo firmware instead.\n'
      + 'To set up the compile server, see the documentation.',
  };
}

/**
 * Basic client-side syntax checks.
 */
function validateSyntax(source: string): CompileError[] {
  const errors: CompileError[] = [];
  const lines = source.split('\n');

  // Check for unmatched braces
  let braceCount = 0;
  for (let i = 0; i < lines.length; i++) {
    for (const ch of lines[i]) {
      if (ch === '{') braceCount++;
      if (ch === '}') braceCount--;
    }
    if (braceCount < 0) {
      errors.push({ line: i + 1, column: 1, message: 'Unexpected closing brace', severity: 'error' });
      break;
    }
  }
  if (braceCount > 0) {
    errors.push({ line: lines.length, column: 1, message: `Missing ${braceCount} closing brace(s)`, severity: 'error' });
  }

  return errors;
}

/** Example sketches shipped with the editor (Arduino API). */
export const EXAMPLE_SKETCHES: { name: string; source: string; language?: 'c' | 'cpp' | 'arduino' }[] = [
  {
    name: 'Blink',
    language: 'arduino',
    source: `// Blink — the classic Arduino sketch
// Toggles LED on pin 5 (PA5 on STM32F103)

void setup() {
  pinMode(5, OUTPUT);
}

void loop() {
  digitalWrite(5, HIGH);
  delay(500);
  digitalWrite(5, LOW);
  delay(500);
}
`,
  },
  {
    name: 'Button + LED',
    language: 'arduino',
    source: `// Read a button and control an LED
// Button on pin 32+13 = PC13, LED on pin 5 = PA5

#define BUTTON_PIN 45  // PC13 (32 + 13)
#define LED_PIN    5   // PA5

void setup() {
  pinMode(LED_PIN, OUTPUT);
  pinMode(BUTTON_PIN, INPUT_PULLUP);
}

void loop() {
  int state = digitalRead(BUTTON_PIN);
  if (state == LOW) {
    digitalWrite(LED_PIN, HIGH);
  } else {
    digitalWrite(LED_PIN, LOW);
  }
}
`,
  },
  {
    name: 'Serial Hello',
    language: 'arduino',
    source: `// Serial communication example
// Prints "Hello LabWired!" and echoes received characters

void setup() {
  Serial_begin(115200);
  Serial_println("Hello LabWired!");
  Serial_println("Type something...");
}

void loop() {
  if (Serial_available()) {
    int ch = Serial_read();
    char buf[2] = { (char)ch, 0 };
    Serial_print("Echo: ");
    Serial_println(buf);
  }
  delay(10);
}
`,
  },
  {
    name: 'Analog Read',
    language: 'arduino',
    source: `// Read analog value from a potentiometer
// Potentiometer on PA0 (pin 0), LED on PA5 (pin 5)

void setup() {
  pinMode(5, OUTPUT);
  Serial_begin(115200);
}

void loop() {
  int val = analogRead(0);
  Serial_print("ADC: ");
  Serial_println_int(val);

  // Turn LED on if value > half
  if (val > 2048) {
    digitalWrite(5, HIGH);
  } else {
    digitalWrite(5, LOW);
  }
  delay(200);
}
`,
  },
  {
    name: 'LED Fade',
    language: 'arduino',
    source: `// Simulate LED fade using rapid toggling
// LED on PA5 (pin 5)

void setup() {
  pinMode(5, OUTPUT);
}

void loop() {
  // Ramp up
  for (int i = 0; i < 100; i++) {
    digitalWrite(5, HIGH);
    delayMicroseconds(i * 10);
    digitalWrite(5, LOW);
    delayMicroseconds((100 - i) * 10);
  }
  // Ramp down
  for (int i = 100; i > 0; i--) {
    digitalWrite(5, HIGH);
    delayMicroseconds(i * 10);
    digitalWrite(5, LOW);
    delayMicroseconds((100 - i) * 10);
  }
}
`,
  },
];
