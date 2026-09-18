/*
 * LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 */
/*
 * LabWired NUCLEO-U575ZI-Q Cube HAL smoke.
 * Register flow mirrors STM32CubeU5
 * Projects/NUCLEO-U575ZI-Q/Templates/TrustZoneDisabled (TrustZone factory-
 * disabled per UM2883), trimmed to HAL-core + GPIO + UART (no BSP).
 *
 * Console: USART1 PA9/PA10 @115200 8N1 (board VCP).
 * LED: PC7 (LD1 green, active high).
 * Output: "U575-HAL OK" once, then "BLINK n LD1=<0|1>" every 250 ms.
 */
#include "stm32u5xx_hal.h"

/* -nostdlib build: same stub as the H5 sibling firmware (no CRT/init arrays). */
void __libc_init_array(void) {}

static UART_HandleTypeDef huart1;

static void SystemClock_Config(void);
static void CACHE_Enable(void);
static void GPIO_Init(void);
static void USART1_Init(void);
static void Error_Handler(void);

int main(void)
{
  char buf[32];
  uint32_t n = 0;

  HAL_Init();
  CACHE_Enable();
  SystemClock_Config();
  GPIO_Init();
  USART1_Init();

  HAL_UART_Transmit(&huart1, (uint8_t *)"U575-HAL OK\r\n", 13, 1000);

  while (1)
  {
    HAL_GPIO_TogglePin(GPIOC, GPIO_PIN_7);
    int level = (HAL_GPIO_ReadPin(GPIOC, GPIO_PIN_7) == GPIO_PIN_SET) ? 1 : 0;
    int len = 0;
    buf[len++] = 'B'; buf[len++] = 'L'; buf[len++] = 'I'; buf[len++] = 'N'; buf[len++] = 'K';
    buf[len++] = ' ';
    buf[len++] = (char)('0' + (n % 10));
    buf[len++] = ' ';
    buf[len++] = 'L'; buf[len++] = 'D'; buf[len++] = '1'; buf[len++] = '=';
    buf[len++] = (char)('0' + level);
    buf[len++] = '\r'; buf[len++] = '\n';
    HAL_UART_Transmit(&huart1, (uint8_t *)buf, (uint16_t)len, 1000);
    n++;
    HAL_Delay(250);
  }
}

static void SystemClock_Config(void)
{
  RCC_OscInitTypeDef RCC_OscInitStruct = {0};
  RCC_ClkInitTypeDef RCC_ClkInitStruct = {0};

  __HAL_RCC_PWR_CLK_ENABLE();
  HAL_PWREx_ControlVoltageScaling(PWR_REGULATOR_VOLTAGE_SCALE1);
  HAL_PWREx_ConfigSupply(PWR_SMPS_SUPPLY);
  __HAL_RCC_PWR_CLK_DISABLE();

  RCC_OscInitStruct.OscillatorType = RCC_OSCILLATORTYPE_MSI;
  RCC_OscInitStruct.MSIState = RCC_MSI_ON;
  RCC_OscInitStruct.MSIClockRange = RCC_MSIRANGE_4; /* 4 MHz */
  RCC_OscInitStruct.MSICalibrationValue = RCC_MSICALIBRATION_DEFAULT;
  RCC_OscInitStruct.PLL.PLLState = RCC_PLL_ON;
  RCC_OscInitStruct.PLL.PLLSource = RCC_PLLSOURCE_MSI;
  RCC_OscInitStruct.PLL.PLLMBOOST = RCC_PLLMBOOST_DIV1;
  RCC_OscInitStruct.PLL.PLLM = 1;
  RCC_OscInitStruct.PLL.PLLN = 80;
  RCC_OscInitStruct.PLL.PLLR = 2;
  RCC_OscInitStruct.PLL.PLLP = 2;
  RCC_OscInitStruct.PLL.PLLQ = 2;
  RCC_OscInitStruct.PLL.PLLFRACN = 0;
  RCC_OscInitStruct.PLL.PLLRGE = RCC_PLLVCIRANGE_0;
  if (HAL_RCC_OscConfig(&RCC_OscInitStruct) != HAL_OK)
  {
    Error_Handler();
  }

  RCC_ClkInitStruct.ClockType = (RCC_CLOCKTYPE_SYSCLK | RCC_CLOCKTYPE_HCLK |
                                 RCC_CLOCKTYPE_PCLK1 | RCC_CLOCKTYPE_PCLK2 |
                                 RCC_CLOCKTYPE_PCLK3);
  RCC_ClkInitStruct.SYSCLKSource = RCC_SYSCLKSOURCE_PLLCLK;
  RCC_ClkInitStruct.AHBCLKDivider = RCC_SYSCLK_DIV1;
  RCC_ClkInitStruct.APB1CLKDivider = RCC_HCLK_DIV1;
  RCC_ClkInitStruct.APB2CLKDivider = RCC_HCLK_DIV1;
  RCC_ClkInitStruct.APB3CLKDivider = RCC_HCLK_DIV1;
  if (HAL_RCC_ClockConfig(&RCC_ClkInitStruct, FLASH_LATENCY_4) != HAL_OK)
  {
    Error_Handler();
  }
}

static void CACHE_Enable(void)
{
  HAL_ICACHE_ConfigAssociativityMode(ICACHE_1WAY);
  HAL_ICACHE_Enable();
}

static void GPIO_Init(void)
{
  GPIO_InitTypeDef gpio = {0};
  __HAL_RCC_GPIOC_CLK_ENABLE();
  gpio.Pin = GPIO_PIN_7;
  gpio.Mode = GPIO_MODE_OUTPUT_PP;
  gpio.Pull = GPIO_NOPULL;
  gpio.Speed = GPIO_SPEED_FREQ_LOW;
  HAL_GPIO_Init(GPIOC, &gpio);
  HAL_GPIO_WritePin(GPIOC, GPIO_PIN_7, GPIO_PIN_RESET);
}

static void USART1_Init(void)
{
  GPIO_InitTypeDef gpio = {0};
  __HAL_RCC_GPIOA_CLK_ENABLE();
  __HAL_RCC_USART1_CLK_ENABLE();

  gpio.Pin = GPIO_PIN_9 | GPIO_PIN_10;
  gpio.Mode = GPIO_MODE_AF_PP;
  gpio.Pull = GPIO_PULLUP;
  gpio.Speed = GPIO_SPEED_FREQ_HIGH;
  gpio.Alternate = GPIO_AF7_USART1;
  HAL_GPIO_Init(GPIOA, &gpio);

  huart1.Instance = USART1;
  huart1.Init.BaudRate = 115200;
  huart1.Init.WordLength = UART_WORDLENGTH_8B;
  huart1.Init.StopBits = UART_STOPBITS_1;
  huart1.Init.Parity = UART_PARITY_NONE;
  huart1.Init.Mode = UART_MODE_TX_RX;
  huart1.Init.HwFlowCtl = UART_HWCONTROL_NONE;
  huart1.Init.OverSampling = UART_OVERSAMPLING_16;
  huart1.Init.OneBitSampling = UART_ONE_BIT_SAMPLE_DISABLE;
  huart1.Init.ClockPrescaler = UART_PRESCALER_DIV1;
  huart1.AdvancedInit.AdvFeatureInit = UART_ADVFEATURE_NO_INIT;
  if (HAL_UART_Init(&huart1) != HAL_OK)
  {
    Error_Handler();
  }
}

void HAL_UART_MspInit(UART_HandleTypeDef *huart)
{
  (void)huart;
}

void SysTick_Handler(void)
{
  HAL_IncTick();
}

static void Error_Handler(void)
{
  __disable_irq();
  while (1)
  {
  }
}

#ifdef USE_FULL_ASSERT
void assert_failed(uint8_t *file, uint32_t line)
{
  (void)file;
  (void)line;
  while (1)
  {
  }
}
#endif
