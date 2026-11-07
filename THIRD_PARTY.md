# Protocol and design references

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
