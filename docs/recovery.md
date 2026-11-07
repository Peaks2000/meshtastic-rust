# Recovery

1. Put the board into its recovery mode: ESP32 usually uses **BOOT + Reset**;
   UF2 boards usually use a double press of **Reset**. Follow your board's instructions.
2. Open **Flashing tool → Detect recovery devices** and select the detected device.
3. Choose the original application firmware file for your exact board, then flash.

ESP32 recovery does not require a normal Meshtastic connection. It backs up the
existing flash, checks the chip/MAC and partition table, and verifies the application
write. **Factory reset + flash** can also clear configuration that causes a bootloop.

UF2 recovery can handle missing or corrupt application firmware when the installed
bootloader identifies a compatible application address/SoftDevice. Available old
firmware and bootloader metadata are saved before copying the update.

Unknown DFU protocols, secure/encrypted ESP32 devices, and damaged partition tables
are not written. A working ROM/UF2 bootloader is required; this does not repair
hardware damage or every possible boot failure. Recovery writes are tested with
simulated hardware, not a physical board.
