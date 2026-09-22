# Release readiness review, 2026-09-22

This record is for the current unreleased source review. It does not amend the
historical rc.33 publication record or assert a new release.

## Owner observations received

| Platform | Observation                                                                                                         | Attribution and limit                                                                                                                                                                               |
| -------- | ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Android  | Local models loaded and performed inference on a physical phone; the app was otherwise in good shape.               | Owner-reported manual evidence. Phone, ABI, OS, model identity, build SHA, version, time, and test steps were not captured. It cannot be tied to a specific commit. Android llama.cpp remains Beta. |
| Linux    | The app worked with and without Ollama. Clicking chat's attachment button froze or crashed it at least twice.       | Owner-reported manual evidence. Exact build and display server were not captured. Treat the attachment interaction as a release blocker until a repaired packaged interaction is exercised.         |
| Windows  | The MSI installed, but launch opened a console behind a black/unusable window that then crashed or stopped working. | Owner-reported manual evidence. Exact build and Windows machine details were not captured. Windows usability remains failed until a packaged Windows build is visibly qualified.                    |

## Review findings in progress

- The published rc.33 MSI was downloaded and extracted on Linux. Its Juniper
  executable is a PE32+ x64 `Windows CUI` subsystem executable (`3`), matching
  the observed extra console. This is independent of the black-window symptom.
- Both native file picker commands used `blocking_pick_file()` through a
  synchronous Tauri command. The locked Tauri command wrapper executes a
  synchronous command inline; the locked dialog plugin warns that its blocking
  picker can deadlock the main event loop. Under Xephyr/X11, the published
  rc.33 AppImage (`SHA-256 c460c47914576a2eb3098f5d6e6a9972c351f922e2c23c9a22c3c952eaefe090`)
  reported frontend readiness, but the attachment click produced no native
  dialog within the probe's 15-second deadline. The probe failed as intended.
  A locally built repaired AppImage (`SHA-256 996600014e39066bd25237f48cd5dd589bace629a6e7675b76c6b88412005744`)
  opened the native dialog, cancelled it, and navigated to Settings afterward;
  982 of 10,370 sampled content pixels changed. The final local AppImage build
  with the native single-picker guard (`SHA-256 dd3b598197066c650147afd4f7d1ca9f0a75ac531bf915c21f06aea68562d3d1`)
  passed the same probe, including rapid double activation with exactly one
  dialog observed. The same build then staged a valid text file through the
  native dialog; 537 of 1,926 sampled composer pixels changed and a visible
  attachment chip appeared. The screenshots before attachment, after valid
  selection, and after cancellation and navigation are in
  `docs/qualification/evidence/`. An unchanged-capture negative control fails
  the image-difference gate.
- The published rc.33 Windows smoke asserted startup logs and process survival
  without window pixels. A black but alive window could pass.

## Current qualification state

This record remains **NOT APPROVED** while the repaired Windows MSI has not
completed native visible-window checks, and Linux's other package modes and
GGUF dialog have not completed their qualification.
No new tag or release is authorized by this record.
