#!/usr/bin/env bash
ELF=target/xtensa-esp32s3-none-elf/debug/hello-rustweek

exec probe-rs gdb --chip esp32s3 --reset-halt "$ELF"
