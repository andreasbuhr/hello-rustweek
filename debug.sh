#!/usr/bin/env bash
set -e

OPENOCD=~/.espressif/tools/openocd-esp32/v0.12.0-esp32-20250707/openocd-esp32/bin/openocd
GDB=~/.espressif/tools/xtensa-esp-elf-gdb/16.3_20250913/xtensa-esp-elf-gdb/bin/xtensa-esp32s3-elf-gdb
ELF=target/xtensa-esp32s3-none-elf/debug/hello-rustweek
SCRIPTS=~/.espressif/tools/openocd-esp32/v0.12.0-esp32-20250707/openocd-esp32/share/openocd/scripts

cargo build

# Flash via espflash (avoids the GDB stub working-memory issue)
espflash flash --chip esp32s3 "$ELF"

"$OPENOCD" -f board/esp32s3-builtin.cfg -s "$SCRIPTS" \
  -c "gdb_memory_map disable" &
OPENOCD_PID=$!
trap "kill $OPENOCD_PID 2>/dev/null" EXIT

sleep 2

"$GDB" "$ELF" -x openocd.gdb
