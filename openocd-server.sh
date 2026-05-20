#!/usr/bin/env bash
OPENOCD=~/.espressif/tools/openocd-esp32/v0.12.0-esp32-20250707/openocd-esp32/bin/openocd
SCRIPTS=~/.espressif/tools/openocd-esp32/v0.12.0-esp32-20250707/openocd-esp32/share/openocd/scripts

exec "$OPENOCD" -f board/esp32s3-builtin.cfg -s "$SCRIPTS" \
  -c "gdb_memory_map disable"
