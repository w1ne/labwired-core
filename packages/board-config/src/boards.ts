export interface BoardCatalogEntry {
  id: string;
  name: string;
  description: string;
  board: string;
  target: string;
  mcu_component_type: string;
  languages: string[];
}

export const PLAYGROUND_BOARD_CATALOG: BoardCatalogEntry[] = [
  {
    id: 'stm32f103-blinky',
    name: 'STM32F103 Blinky',
    description: 'Catalog starter lab: STM32F103 with an LED on PA5.',
    board: 'stm32f103',
    target: 'stm32f103',
    mcu_component_type: 'stm32-dev',
    languages: ['c', 'rust'],
  },
  {
    id: 'nucleo-f401re',
    name: 'Nucleo-F401RE',
    description: 'Catalog board: STM32F401RE Nucleo with LED on PA5 and user button on PC13.',
    board: 'stm32f401',
    target: 'stm32f401',
    mcu_component_type: 'nucleo-f401re',
    languages: ['c', 'cpp'],
  },
  {
    id: 'stm32f401cdu6-blackpill',
    name: 'STM32F401CDU6 Black Pill',
    description: 'Catalog board: compact STM32F401CDU6 Black Pill with active-low PC13 LED.',
    board: 'stm32f401cdu6',
    target: 'stm32f401cdu6',
    mcu_component_type: 'stm32-blackpill',
    languages: ['c', 'cpp'],
  },
  {
    id: 'nucleo-h563zi',
    name: 'Nucleo-H563ZI',
    description: 'Catalog board: STM32H563ZI Nucleo-144 with LEDs and user button.',
    board: 'stm32h563',
    target: 'stm32h563',
    mcu_component_type: 'nucleo-h563zi',
    languages: ['c', 'cpp'],
  },
  {
    id: 'rp2040-pico',
    name: 'Raspberry Pi Pico',
    description: 'Catalog board: RP2040 Pico.',
    board: 'rp2040',
    target: 'rp2040',
    mcu_component_type: 'rpi-pico',
    languages: ['c', 'cpp', 'rust'],
  },
  {
    id: 'nrf52840-dk',
    name: 'nRF52840 DK',
    description: 'Catalog board: Nordic nRF52840 development kit.',
    board: 'nrf52840',
    target: 'nrf52840',
    mcu_component_type: 'nrf52840-dk',
    languages: ['c', 'rust'],
  },
  {
    id: 'esp32c3-supermini',
    name: 'ESP32-C3 Super Mini',
    description: 'Catalog board: ESP32-C3 Super Mini with built-in LED on GPIO8.',
    board: 'esp32c3',
    target: 'esp32c3',
    mcu_component_type: 'esp32-c3-supermini',
    languages: ['c', 'cpp'],
  },
  {
    id: 'esp32s3-zero',
    name: 'ESP32-S3-Zero',
    description: 'Catalog board: ESP32-S3-Zero with RGB LED on GPIO48.',
    board: 'esp32s3',
    target: 'esp32s3',
    mcu_component_type: 'esp32-s3-zero',
    languages: ['c', 'cpp'],
  },
];

export function listPlaygroundBoards(filter?: string): BoardCatalogEntry[] {
  if (!filter) return PLAYGROUND_BOARD_CATALOG;
  const q = filter.toLowerCase();
  return PLAYGROUND_BOARD_CATALOG.filter((board) =>
    [board.id, board.name, board.description, board.board, board.target, board.mcu_component_type]
      .some((value) => value.toLowerCase().includes(q)),
  );
}

export function getPlaygroundBoard(id: string): BoardCatalogEntry | undefined {
  return PLAYGROUND_BOARD_CATALOG.find((board) => board.id === id);
}
