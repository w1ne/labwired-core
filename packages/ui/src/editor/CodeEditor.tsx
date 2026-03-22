import { useRef, useCallback } from 'react';
import Editor, { type OnMount } from '@monaco-editor/react';
import type { editor, languages, IPosition } from 'monaco-editor';

export interface CompileError {
  line: number;
  column: number;
  message: string;
  severity: 'error' | 'warning';
}

interface CodeEditorProps {
  source: string;
  language?: string;
  onChange: (source: string) => void;
  errors?: CompileError[];
  readOnly?: boolean;
}

export function CodeEditor({
  source,
  language = 'c',
  onChange,
  errors = [],
  readOnly = false,
}: CodeEditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Parameters<OnMount>[1] | null>(null);

  const handleMount: OnMount = useCallback((ed, monaco) => {
    editorRef.current = ed;
    monacoRef.current = monaco;

    // Configure C/C++ language
    monaco.languages.register({ id: 'c' });
    monaco.languages.register({ id: 'cpp' });

    // Arduino-style keyword completions
    const ARDUINO_KEYWORDS = [
      'void', 'int', 'char', 'float', 'double', 'long', 'unsigned', 'signed',
      'const', 'static', 'volatile', 'extern', 'return', 'if', 'else', 'for',
      'while', 'do', 'switch', 'case', 'break', 'continue', 'default',
      'struct', 'typedef', 'enum', 'sizeof', 'include', 'define', 'ifdef',
      'ifndef', 'endif', 'pragma',
    ];

    const ARDUINO_FUNCTIONS = [
      'setup', 'loop', 'pinMode', 'digitalWrite', 'digitalRead',
      'analogRead', 'analogWrite', 'delay', 'delayMicroseconds',
      'millis', 'micros', 'Serial', 'Wire', 'SPI',
      'HIGH', 'LOW', 'INPUT', 'OUTPUT', 'INPUT_PULLUP',
      'LED_BUILTIN', 'A0', 'A1', 'A2', 'A3', 'A4', 'A5',
      'HAL_GPIO_WritePin', 'HAL_GPIO_ReadPin', 'HAL_GPIO_Init',
      'HAL_UART_Transmit', 'HAL_UART_Receive',
      'HAL_Delay', 'HAL_GetTick',
      'GPIO_PIN_SET', 'GPIO_PIN_RESET',
      'GPIOA', 'GPIOB', 'GPIOC',
    ];

    monaco.languages.registerCompletionItemProvider('c', {
      provideCompletionItems: (_model: editor.ITextModel, position: IPosition): languages.ProviderResult<languages.CompletionList> => {
        const word = _model.getWordUntilPosition(position);
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        };
        const suggestions = [
          ...ARDUINO_KEYWORDS.map((kw) => ({
            label: kw,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: kw,
            range,
          })),
          ...ARDUINO_FUNCTIONS.map((fn) => ({
            label: fn,
            kind: monaco.languages.CompletionItemKind.Function,
            insertText: fn,
            range,
          })),
        ];
        return { suggestions };
      },
    });

    // Dark theme
    monaco.editor.defineTheme('labwired-dark', {
      base: 'vs-dark',
      inherit: true,
      rules: [
        { token: 'keyword', foreground: '569cd6', fontStyle: 'bold' },
        { token: 'type', foreground: '4ec9b0' },
        { token: 'string', foreground: 'ce9178' },
        { token: 'comment', foreground: '6a9955' },
        { token: 'number', foreground: 'b5cea8' },
      ],
      colors: {
        'editor.background': '#1a1a2e',
        'editor.foreground': '#d4d4d4',
        'editorLineNumber.foreground': '#858585',
        'editor.selectionBackground': '#264f78',
        'editor.lineHighlightBackground': '#2a2a4a',
      },
    });
    monaco.editor.setTheme('labwired-dark');
  }, []);

  // Update error markers when errors change
  const prevErrorsRef = useRef<CompileError[]>([]);
  if (errors !== prevErrorsRef.current && monacoRef.current && editorRef.current) {
    prevErrorsRef.current = errors;
    const model = editorRef.current.getModel();
    if (model) {
      const monaco = monacoRef.current;
      const markers: editor.IMarkerData[] = errors.map((err) => ({
        severity: err.severity === 'error'
          ? monaco.MarkerSeverity.Error
          : monaco.MarkerSeverity.Warning,
        startLineNumber: err.line,
        startColumn: err.column,
        endLineNumber: err.line,
        endColumn: err.column + 100,
        message: err.message,
      }));
      monaco.editor.setModelMarkers(model, 'compile', markers);
    }
  }

  return (
    <div className="code-editor-container" style={{ width: '100%', height: '100%' }}>
      <Editor
        defaultLanguage={language}
        value={source}
        onChange={(v) => onChange(v ?? '')}
        onMount={handleMount}
        options={{
          readOnly,
          fontSize: 14,
          fontFamily: "'JetBrains Mono', 'Fira Code', 'Consolas', monospace",
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          lineNumbers: 'on',
          renderWhitespace: 'selection',
          tabSize: 2,
          automaticLayout: true,
          suggestOnTriggerCharacters: true,
          quickSuggestions: true,
          wordWrap: 'off',
          folding: true,
          bracketPairColorization: { enabled: true },
        }}
      />
    </div>
  );
}
