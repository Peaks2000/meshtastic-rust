# Protocol and design references

* Official Meshtastic app icon (`logo/logo.svg`) and green wordmark
  (`typelogo/typelogo-green.svg`): <https://github.com/meshtastic/design>
  at commit `fbb51646b6c25276b0a4c9ee4309fe116e967a52`, GPL-3.0.
  The SVG artwork is included unchanged in `assets/`. Meshtastic logos remain
  trademarks of Meshtastic LLC: <https://meshtastic.org/docs/legal/licensing-and-trademark/>.

* Official Meshtastic protobufs: <https://github.com/meshtastic/protobufs>
  at commit `85f3f4357946a52daa5310285476d2dffb137c7b`.
  The `.proto` sources and their GPL-3.0 license are vendored in `vendor/protobufs/`.
* Apple client settings categories and field coverage were checked against
  <https://github.com/meshtastic/Meshtastic-Apple> at commit
  `50aaa887be2e64415f5f4b5144c532d25378f8ae`.
  Swift application code is not included in this client.
* Wire framing and BLE UUIDs follow the official protocol and transport references:
  <https://github.com/meshtastic/rust> and <https://meshtastic.org/docs/development/>.
* ESP32 flashing uses the official Espressif esptool:
  <https://github.com/espressif/esptool>.

Meshtastic is a trademark of Meshtastic LLC. This is an independent desktop client.
