#!/usr/bin/env bash
ELF=target/xtensa-esp32s3-none-elf/debug/hello-rustweek

exec probe-rs dap-server --port 3333
