/****************************************************************************
 * curiosity-pic32mz-ef/src/pic32mz_bringup.c
 *
 * SPDX-License-Identifier: Apache-2.0
 *
 * Derived from the Apache NuttX 12.11.0 flipnclick-pic32mz board
 * (boards/mips/pic32mz/flipnclick-pic32mz), retargeted to the Microchip
 * Curiosity PIC32MZ EF 2.0 (DM320209, PIC32MZ2048EFM144).
 *
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.  The
 * ASF licenses this file to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance with the
 * License.  You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
 * WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
 * License for the specific language governing permissions and limitations
 * under the License.
 *
 ****************************************************************************/

#include <nuttx/config.h>

#include <sys/types.h>
#include <syslog.h>

#include <nuttx/fs/fs.h>
#include <nuttx/leds/userled.h>

#include "curiosity-pic32mz-ef.h"

int pic32mz_bringup(void)
{
  int ret = OK;

#ifdef CONFIG_FS_PROCFS
  ret = nx_mount(NULL, "/proc", "procfs", 0, NULL);
  if (ret < 0)
    {
      syslog(LOG_ERR, "ERROR: Failed to mount procfs at /proc: %d\n", ret);
    }
#endif

#ifdef CONFIG_USERLED_LOWER
  ret = userled_lower_initialize("/dev/userleds");
  if (ret < 0)
    {
      syslog(LOG_ERR, "ERROR: userled_lower_initialize() failed: %d\n", ret);
    }
#endif

  UNUSED(ret);
  return OK;
}
