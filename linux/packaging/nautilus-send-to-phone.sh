#!/bin/sh
# Nautilus script: right-click → Scripts → "Send with AirBridge".
# Opens AirBridge with the selected files ready; click a device to send.
exec airbridge send "$@"
