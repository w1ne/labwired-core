#pragma once
#define HICK_VALUE 8000000u
#define HEXT_VALUE 8000000u
#define HEXT_STARTUP_TIMEOUT 0x3000u
#include "at32f421.h"
#define CRM_MODULE_ENABLED
#define GPIO_MODULE_ENABLED
#define USART_MODULE_ENABLED
#define FLASH_MODULE_ENABLED
#define TMR_MODULE_ENABLED
#define DMA_MODULE_ENABLED
#include "at32f421_crm.h"
#include "at32f421_gpio.h"
#include "at32f421_usart.h"
#include "at32f421_flash.h"
#include "at32f421_tmr.h"
#include "at32f421_dma.h"
