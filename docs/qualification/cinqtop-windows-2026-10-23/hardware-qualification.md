# CINQTOP hardware qualification (2026-10-23)

CINQTOP is the secondary Cinqic Windows workstation used for the 0.4.0-Alpha
Windows qualification recorded in this folder. Everything below was measured
on the machine with Windows PowerShell 5.1 and CIM/WMI on 2026-10-23 before
any Juniper software or build toolchain was installed. The machine-readable
form is [cinqtop-inventory.json](cinqtop-inventory.json). Serial numbers,
MAC addresses and user paths are deliberately omitted.

## Verified specification

| Component        | Measured                                                                                                                                      |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Machine          | HP Laptop 14-dq6xxx, SKU B86L6UA#ABA, board HP 8E05, AMI firmware F.12 (2026-03-05)                                                           |
| Processor        | Intel N150 (Family 6 Model 190), 4 cores / 4 threads, no SMT, 2 MiB L2, 6 MiB L3                                                              |
| Instruction sets | SSE4.2, AVX, AVX2 present; AVX-512 absent (`IsProcessorFeaturePresent`)                                                                       |
| Memory           | one 16 GiB DDR4-3200 SODIMM, single channel; 16,818,876,416 bytes usable (about 344 MiB firmware-reserved)                                    |
| Graphics         | Intel integrated graphics (PCI 8086:46D4), driver 32.0.101.7084 (2026-01-14), shared memory only                                              |
| Display          | 1366x768 at 59 Hz, 96 DPI (100% scaling), single internal panel                                                                               |
| Storage          | Samsung KLUDG4UHGC-B0E1, UFS, 128 GB class (127,984,992,256 bytes), GPT; `C:` NTFS 118.0 GiB with 57.1 GiB free before toolchain installation |
| Operating system | Windows 11 Home 25H2, build 26200.9711, x64, installed 2026-09-17                                                                             |
| WebView2         | Evergreen runtime 153.0.4234.32 already present (Edge 153 installed)                                                                          |
| Network          | Realtek 8852BE-VT Wi-Fi 6 only; no wired adapter                                                                                              |
| Battery          | 36,775 mWh full-charge capacity; on battery at 44% during capture                                                                             |
| Power plan       | Balanced                                                                                                                                      |

The SMBIOS memory array reports an 8 GiB maximum while the installed 16 GiB
module is fully usable; the SMBIOS maximum is not reliable on this board.
`VirtualizationFirmwareEnabled` reads false while a hypervisor is present
(Windows virtualization-based security), so firmware virtualization support
cannot be concluded from WMI alone.

## Software environment before the assignment

Installed by the owner: Brave 156, the Claude desktop app, Microsoft 365 Apps
and Microsoft Edge 153 (which carries the WebView2 runtime). No Git, Node.js,
pnpm, Rust, MSVC, CMake or Python existed; the `python` command was only the
Microsoft Store alias stub. Startup entries were limited to the HP System
Event Utility host, Edge session-start, Windows Security tray and the Realtek
audio service.

## Resource baseline

Five samples at 3-second spacing, 26 minutes after boot, on battery, with the
Claude desktop app running (11 processes, about 2.0 GiB working set) and Brave
closed:

| Metric                | Range                                                                           |
| --------------------- | ------------------------------------------------------------------------------- |
| CPU utilisation       | 22-32% (the Claude app and this session accounted for most of it)               |
| Processor performance | 135-145% of the 800 MHz nominal base, so the cores were boosting, not throttled |
| In-use memory         | 6.7-6.9 GiB of 15.66 GiB; committed 6.0-6.1 GiB; file cache about 1.0 GiB       |
| Available memory      | 9.0-9.2 GiB                                                                     |
| Disk time             | 0-7%                                                                            |
| Page file             | 2,944 MB, 0 MB in use                                                           |

Subtracting the Claude app gives a Windows-only baseline of roughly 4.7-4.9
GiB in use, consistent with the owner's earlier 4.2 GiB post-boot screenshot
once the desktop-search and widget hosts had settled. Brave was not running
during this capture; a Brave profile adds several hundred MiB per window.

## Budget for this assignment

- Memory: with Windows, Claude and Brave resident, about 7-8 GiB remains for
  Juniper, its `llama-server` child and a model. Test models should keep the
  GGUF file at or below 2 GiB (for example a 1-2B instruction model at Q4 or
  Q8) so the model, KV cache and WebView2 stay well under that figure. Models
  above 4 GiB are out of budget on this machine.
- CPU: four efficiency cores with AVX2. Expect single-digit tokens per second
  on a 1-2B model and slow Rust compilation; keep `cargo` and the llama.cpp
  build at two parallel jobs when anything else is running.
- Storage: 57 GiB free before tooling. Reserve about 15 GiB for Visual Studio
  Build Tools, Rust, Node.js, `node_modules`, the Tauri target directory and
  the llama.cpp build; keep at least 20 GiB free for Windows updates and
  application data.
- Graphics: integrated only. No CUDA or discrete GPU; the CPU `llama-server`
  variant is the only applicable runtime and `GGML_NATIVE=OFF` builds must
  rely on their run-time CPU dispatch for AVX2.
- Display: a single 1366x768 panel at 100% scaling. The configured 1280x820
  Juniper window does not fit vertically, so window placement and minimum
  size behaviour matter here.
- Power: the capture ran on battery. Long builds and inference measurements
  should run on mains power; battery runs are recorded as such.

## Smart App Control

Windows 11 Home on CINQTOP had Smart App Control in evaluation mode when the
run started and switched it on at about 02:58 on 2026-10-23
(`HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy`: `SAC_PreviousState=2`,
`VerifiedAndReputablePolicyState=1`). From then on the machine refuses to run
newly built unsigned executables, which ends local native build-and-run work
until the owner decides about the setting (turning it off is permanent).
Defender real-time protection and tamper protection are on; neither was
changed.

## Known uncertainty

- Thermal zone temperatures are not exposed through `MSAcpi_ThermalZoneTemperature`
  on this firmware, so throttling can only be inferred from the processor
  performance counter.
- The WMI `AdapterRAM` value (2 GiB) is a reporting cap, not the shared memory
  the graphics driver can actually use.
- Battery wear relative to design capacity was not available from WMI.
