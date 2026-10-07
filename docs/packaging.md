# Packaging and Windows integration

Status: wave 1 (W1-D), 2026-10-07. Measured on the development PC in `CLAUDE.md` (Windows 10 22H2, build 19045). Numbers in brackets are sources at the end.

## Build the packages

Run these in Windows PowerShell 5.1. PowerShell 7 is not needed.

1. `C:\Users\Armaan\.cargo\bin\cargo.exe build --release`
2. `powershell -NoProfile -ExecutionPolicy Bypass -File tools\package.ps1`

Options: `-SkipSign` leaves the MSIX unsigned. `-SkipMsix` builds only the ZIP. `-WindowsSdkBin` points to another SDK `bin\<version>\x64` folder.

The script never deletes an earlier output. If `dist\Preview` exists, it adds a UTC time stamp to every name below.

| Output | Contents |
| --- | --- |
| `dist\Preview\` | ZIP payload: exe, `pdfium.dll`, README, notices, `register-file-associations.ps1`, `licenses\`, `manifest.json` (SHA256 per file) |
| `dist\Preview-Windows-x64.zip` | The folder above |
| `dist\Preview-msix-layout\` | MSIX payload: exe, `pdfium.dll`, notices, `licenses\`, `Assets\` (generated logos), `AppxManifest.xml` |
| `dist\Preview-x64.msix` | Packed, schema-checked, and test-signed package |
| `dist\Preview-msix-check\` | The MSIX unpacked again for the file-list check |

What the script does:

1. Checks the PDFium DLL hash against `runtime\x64\provenance.json`.
2. Copies the payload and license texts. It reads single files from the `runtime\` junction and never copies or deletes that folder recursively.
3. Rejects any locked crate with a GPL or AGPL license, then copies each crate's license files.
4. Sets file times before 1980 to now, because ZIP cannot store them (two `quick-error` license files are dated 1970).
5. Builds the ZIP.
6. Draws three placeholder logos with System.Drawing: 44, 50, and 150 px.
7. Copies `packaging\AppxManifest.xml` with `Identity/@Version` set from `Cargo.toml` (0.1.0 becomes 0.1.0.0).
8. Runs `makeappx pack /h SHA256` from SDK 10.0.26100 [1]. Packing fails on a manifest that breaks the schema (measured below).
9. Signs with SignTool and a self-signed certificate in `Cert:\CurrentUser\My` [2][3].
10. Unpacks the MSIX and compares its file list with the layout.

### Measured results (2026-10-07)

| Check | Result |
| --- | --- |
| ZIP size | 4,424,141 bytes (4.2 MiB). Target under 30 MB: met. |
| MSIX size | 4,606,137 bytes (4.4 MiB). Target under 30 MB: met. |
| Schema check works | A copy of the manifest with `MultiSelectModel="Many"` failed: "error C00CE169: App manifest validation error ... 'Many' violates enumeration constraint of 'Player Document Single'". |
| Signature | SignTool: "Successfully signed". `Get-AuthenticodeSignature` reports that the chain ends in an untrusted root. This is expected for a test certificate. |
| Round trip | `makeappx unpack` returns every layout file. |
| Install | Not done. The rules for this slice forbid installing the package or trusting the certificate. |

Sizes exclude the optional AI pack. The release exe in both packages does not yet include `src/integration.rs` (it joins in wave 2).

### Test certificate

The script creates `CN=Preview for Windows Test` in `Cert:\CurrentUser\My` with the code-signing EKU, as Microsoft documents [3]. It needs no admin rights. Later runs reuse it. Windows does not trust it, so the MSIX does not install as is. To remove it:

```powershell
Get-ChildItem Cert:\CurrentUser\My | Where-Object Subject -eq 'CN=Preview for Windows Test' | Remove-Item
```

## Steps that need the owner

1. **Publisher identity.** For the Store, reserve the name in Partner Center and copy its `Identity Name`, `Publisher`, and `PublisherDisplayName` into `packaging\AppxManifest.xml`. The Store signs Store submissions. For a direct download, buy a code-signing certificate whose subject matches `Publisher` exactly [3], then sign with `signtool sign /fd SHA256 /sha1 <thumbprint>` (or `/f <pfx>`) [2].
2. **Store submission.** `runFullTrust` is a restricted capability. Partner Center asks why the app needs it at submission time.
3. **Install test on a test PC.** Import the test certificate into `Cert:\LocalMachine\TrustedPeople` from an admin prompt [3], then `Add-AppxPackage dist\Preview-x64.msix`. Check "Open with", Default apps, and the Explorer verbs. Remove the certificate after the test.
4. **Hosting and winget.** Upload the signed MSIX to an HTTPS URL. In `packaging\winget\`, replace `REPLACE_WITH_HTTPS_URL_OF_Preview-x64.msix`, `REPLACE_WITH_SHA256_OF_THE_MSIX` (`Get-FileHash -Algorithm SHA256`), and `REPLACE_WITH_LICENSE`. Run `winget validate --manifest packaging\winget`, then open a pull request to `microsoft/winget-pkgs` under `manifests\p\PreviewForWindows\PreviewForWindows\<version>\` [4].
5. **Art.** Replace the generated placeholder logos. Scaled logo variants need a `resources.pri` (makepri); the current package has none (unverified whether Windows shows the plain files at every size).
6. **ARM64.** Needs the MSVC ARM64 tools (admin install), then a second package with `ProcessorArchitecture="arm64"`.

### winget template

`packaging\winget\` holds the three manifest files (version, installer, default locale) in schema 1.12.0 [4], with an MSIX installer and the 11 file extensions. Optional fields to add once known: `PackageFamilyName` and `SignatureSha256` [4]. With the placeholders filled (the local MSIX SHA256, a dummy URL, and a license), `winget validate` (winget 1.29.380) printed "Manifest validation succeeded". The unfilled template fails only on the placeholder fields.

## Windows integration (`src/integration.rs`)

The module is not in `src/main.rs` yet. The tests include it with `#[path = "../src/integration.rs"]`. GUI checks stopped on 2026-10-07 at the owner's request, so every check below is headless. Share and drag are "pending GUI check".

| Function | Purpose | Evidence |
| --- | --- | --- |
| `parse_args(args) -> Command` | Reads `[--convert\|--resize\|--combine] path...`; makes paths absolute | `parse_args_reads_verb_and_absolute_paths` |
| `merge(&mut Command, Command) -> bool` | Collects same-action paths without duplicates | `merge_collects_one_action_without_duplicates` |
| `encode(&Command) -> Vec<u16>`, `decode(&[u16]) -> Option<Command>` | WM_COPYDATA payload | `decode_accepts_only_tagged_absolute_paths` |
| `unsafe decode_copydata(LPARAM) -> Option<Command>` | Safe read of a received COPYDATASTRUCT | same test |
| `find_window(class) -> Option<HWND>` | Running instance's top-level or message-only window | `forward_reaches_running_window` |
| `forward(HWND, &Command) -> Result<(), String>` | Sends WM_COPYDATA, passes foreground rights | `forward_reaches_running_window` |
| `hand_off(class, &Command) -> Option<InstanceGuard>` | Single instance: forward, or become the running instance | `hand_off_from_a_new_process_meets_warm_open_target`, `hand_off_opens_its_own_window_when_the_primary_never_answers` |
| `register(exe)`, `unregister(exe)`, `register_at(base, exe)`, `unregister_at(base, exe)` | Per-user file types, Open with, Default apps, Explorer verbs | `register_writes_per_user_keys_and_unregister_removes_them`, `script_writes_the_same_keys_as_rust` |
| `default_apps_uri(build) -> String`, `open_default_apps()`, `windows_build() -> u32` | Default apps link | `default_apps_link_matches_windows_version` (URI only; Settings not opened) |
| `share_files(HWND, &[PathBuf]) -> Result<(), String>` | Windows share sheet | Compiles. Pending GUI check. |
| `file_data_object(&[PathBuf]) -> Result<IDataObject, String>` | CF_HDROP and shell ID lists for files in any folders | `drag_data_object_carries_files_from_two_folders` |
| `drag_files(HWND, &[PathBuf]) -> Result<DROPEFFECT, String>` | OLE drag of copies | Refusal without a pressed button tested. Drag loop pending GUI check. |
| `recent_dir()`, `load_recent(dir)`, `add_recent(dir, path)`, `note_recent(path)` | Recent files and jump list feed | `recent_files_are_newest_first_capped_and_pruned` |

### Single instance

1. The new process calls `hand_off(WINDOW_CLASS, &command)` before it creates a window. It creates the named mutex `Local\PreviewForWindowsMain`.
2. If the mutex is new, this process is the running instance. It keeps the returned guard until exit.
3. If the mutex exists, the process looks for a top-level or message-only window of class `PreviewForWindowsMain` with one `FindWindowEx` call [5]. It waits up to 5 s for the other instance to create its window.
4. It calls `AllowSetForegroundWindow` with the owner's process ID, so the running window can come to the front [6]. Then it sends WM_COPYDATA with `SendMessageTimeoutW` (5 s, abort if hung).
5. The payload is UTF-16 strings, each ending in NUL: the action flag, then absolute paths. `dwData` is `0x50465731` ("PFW1").
6. The receiver copies the data during the message, as Microsoft requires [7]. It rejects a wrong tag, a null pointer, an empty or odd size, over 1 MiB, a missing final NUL, or an unknown action. It keeps only drive and UNC paths, so relative paths and device paths such as `\\.\PhysicalDrive0` are dropped.
7. If the running window does not return TRUE within 5 s, the new process opens the files itself. A request is never lost.

Measured handoff (test binary as the second process, so it has no D2D or WIC imports):

| Measure | Runs | Result |
| --- | --- | --- |
| `forward` send and decode in one process | 6 runs of 200 sends | p95 0.039 to 0.118 ms |
| New process start to WM_COPYDATA received | 6 runs of 30 launches | p50 36 to 78 ms, p95 54 to 102 ms |

The PRD warm-open target is p95 under 150 ms for the whole open [PRD]. The handoff alone uses up to two thirds of it on this PC, almost all in process creation. Measure the real exe again in wave 2.

### File associations and Default apps

`register_at(base, exe)` writes only under `HKEY_CURRENT_USER\<base>` (`Software` in real use), so it needs no admin rights [8]:

| Key under `HKCU\Software` | Values |
| --- | --- |
| `Classes\PreviewForWindows.<Type>` (8 ProgIDs: Pdf, Jpeg, Png, Webp, Heif, Gif, Tiff, Bmp) | Type name, `DefaultIcon`, `shell\open` with `MultiSelectModel=Player` and `"exe" "%1"` [9] |
| `Classes\<ext>\OpenWithProgids` | `<ProgID>` = empty string, for "Open with" [10] |
| `Classes\Applications\preview-for-windows.exe` | `FriendlyAppName`, `SupportedTypes`, `shell\open` |
| `PreviewForWindows\Capabilities` | `ApplicationName`, `ApplicationDescription`, `FileAssociations\<ext>` = ProgID [11] |
| `RegisteredApplications` | `Preview for Windows` = `Software\PreviewForWindows\Capabilities` [11][12] |
| `Classes\SystemFileAssociations\<ext>\shell\PreviewForWindows.<Verb>` | `MUIVerb`, `MultiSelectModel=Player`, `Icon`, `command` [13][14] |

Extensions: .pdf .jpg .jpeg .png .webp .heic .heif .gif .tif .tiff .bmp. The code never writes an extension's default value or `UserChoice`, so it never sets a default; Microsoft says the choice of default should be user driven [11]. `register` and `unregister` then call `SHChangeNotify(SHCNE_ASSOCCHANGED)` [8]. `unregister_at` removes only what `register_at` wrote.

`tools\register-file-associations.ps1` writes the same keys for ZIP users, with `-Unregister`. `script_writes_the_same_keys_as_rust` runs both against scratch keys and compares every value. Tests write only `HKCU\Software\PreviewForWindows-Test` and delete it; the real keys were checked absent after the run.

`open_default_apps` opens `ms-settings:defaultapps` on Windows 10. On build 22000 and later it adds `?registeredAppUser=Preview%20for%20Windows`, which Windows 11 21H2 and 22H2 (with the 2023-04 update) and 23H2 or later support [12].

### Explorer verbs: Convert, Resize, Combine into PDF

Command line: `preview-for-windows.exe --convert "<path>"`, `--resize`, or `--combine`, followed by one or more paths. Convert and Resize apply to the 10 image extensions; Combine into PDF also applies to .pdf. The verbs live under `SystemFileAssociations`, so they stay available when another app is the default [14].

Explorer limits by selection model [15]:

| Verb type | Document | Player |
| --- | --- | --- |
| Legacy (command line) | 15 items | 100 items |
| COM (DropTarget, ExecuteCommand) | 15 items | No limit |

`Player` is the largest limit for a command-line verb: 100 files, which meets the PRD's "100 selected files". A command-line verb "does not enable re-use of an already running process", and its command line is limited to 2,000 characters [16]. So Explorer may start one process per selected file (unverified here, because the rules for this slice forbid registering real Explorer verbs). Each process forwards its part through `hand_off`, and the shell joins the parts with `merge`.

Routes for more than 100 files, evaluated and not implemented:

1. **DropTarget local server (unpackaged).** The exe implements `IDropTarget` as a COM local server, registered under `HKCU\Software\Classes\CLSID`, with a `DropTarget` verb. Microsoft says it reuses a running handler and passes all items in one data object, with no size limit [16][14]. It works on Windows 10 and 11. Not built: it needs a COM server in `main.rs` and a CLSID in the user's registry, which this slice cannot verify.
2. **MSIX `desktop4:FileExplorerContextMenus` with `IExplorerCommand`.** Microsoft documents a native DLL COM server (`com:SurrogateServer`) for the Windows 11 context menu [17]. It needs an installed package to verify. Not built.
3. **MSIX `uap3:Verb` with `MultiSelectModel="Player"`** (in `packaging\AppxManifest.xml`, schema-valid). Microsoft says Player activates the app once with all selected files as arguments [18][19]. Phase 1 research cites an open Windows App SDK issue (#5066) where each of N processes gets all N files. `merge` handles both cases. Runtime behavior is unverified until an install test.

If selections over 100 files matter, build route 1.

### Share (pending GUI check)

`share_files` gets the `DataTransferManager` for the window through `IDataTransferManagerInterop::GetForWindow`, adds a one-time `DataRequested` handler that sets the required title and the `StorageFile` items, and calls `ShowShareUIForWindow` [20]. Files resolve to `StorageFile` on the calling thread. GUI check: open a file, press Share, confirm the sheet lists targets, share to Mail or Nearby sharing, and repeat on Windows 11.

### Drag out (drag loop pending GUI check)

`file_data_object` builds a shell data object from item ID lists, so files from different folders work and Explorer gets CF_HDROP. `drag_files` calls `SHDoDragDrop` with no drop source; the shell then supplies one and the drag image [21]. It allows only `DROPEFFECT_COPY`, so the original never moves. It refuses to start unless a mouse button is down, because OLE would otherwise drop at once under the pointer. The UI thread must call `OleInitialize` first [22]. Microsoft says not to start a drag from a touch or pen handler; start it from the mouse message the system synthesizes [22].

### Recent files

`recent.txt` in `%LOCALAPPDATA%\PreviewForWindows\` holds one full path per line, newest first, at most 20. `load_recent` drops lines for missing files. `add_recent` matches paths without case, writes a temp file, then renames it. `note_recent` also calls `SHAddToRecentDocs(SHARD_PATHW)`, which feeds the Recent list in the app's jump list [23]. Tests use a scratch folder and never call `note_recent`, because it writes the user's real Recent items. Jump list display: pending GUI check.

## Wave 2 wiring

1. Register the main window class as `integration::WINDOW_CLASS` (`PreviewForWindowsMain`). The shell uses `PreviewForWindowsSpeedSpike` today.
2. At start: `let command = integration::parse_args(std::env::args_os().skip(1));` then `let Some(_guard) = integration::hand_off(integration::WINDOW_CLASS, &command) else { return };`. Keep the guard alive until exit.
3. In the window procedure, on WM_COPYDATA: `if let Some(command) = unsafe { integration::decode_copydata(lparam) } { ...; return LRESULT(1) }`, else return 0. Restore the window if it is minimized and call `SetForegroundWindow`. For Open, add tabs and skip paths already open. For Convert, Resize, and Combine, `merge` commands that arrive within a short window (suggested 300 ms, unmeasured), then start one job.
4. Call `OleInitialize` instead of `CoInitializeEx` on the UI thread, for `drag_files`.
5. Call `note_recent` after each successful open. Show `load_recent` in the empty window.
6. First run: offer to become the default. On yes, call `register(current_exe)` and then `open_default_apps()`.
7. Share button and right-click Share: `share_files(hwnd, &[path])`.

## Sources

1. Create an app package with the MakeAppx.exe tool: https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool
2. Sign an app package using SignTool: https://learn.microsoft.com/en-us/windows/msix/package/sign-app-package-using-signtool
3. Create a certificate for package signing: https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing
4. Create your package manifest (winget): https://learn.microsoft.com/en-us/windows/package-manager/package/manifest
5. Window features, message-only windows: https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#message-only-windows
6. AllowSetForegroundWindow: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow
7. WM_COPYDATA: https://learn.microsoft.com/en-us/windows/win32/dataxchg/wm-copydata
8. File types (per-user `HKCU\Software\Classes`, `SHChangeNotify`): https://learn.microsoft.com/en-us/windows/win32/shell/fa-file-types
9. Programmatic identifiers: https://learn.microsoft.com/en-us/windows/win32/shell/fa-progids
10. How to include an application in the Open With dialog box: https://learn.microsoft.com/en-us/windows/win32/shell/how-to-include-an-application-on-the-open-with-dialog-box
11. Default Programs (Capabilities, RegisteredApplications, user-driven defaults): https://learn.microsoft.com/en-us/windows/win32/shell/default-programs
12. Launch the Default Apps settings page: https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-default-apps-settings
13. Creating shortcut menu handlers (MUIVerb, per-user verbs): https://learn.microsoft.com/en-us/windows/win32/shell/context-menu-handlers
14. PerceivedTypes, SystemFileAssociations, and Application Registration: https://learn.microsoft.com/en-us/previous-versions//bb776871(v=vs.85)
15. How to employ the verb selection model: https://learn.microsoft.com/en-us/windows/win32/shell/how-to-employ-the-verb-selection-model
16. Choosing a static or dynamic shortcut menu method: https://learn.microsoft.com/en-us/windows/win32/shell/shortcut-choose-method
17. Add a File Explorer context menu command to a packaged desktop app: https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer
18. Integrate your desktop app with Windows using packaging extensions: https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions
19. uap3:Verb: https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-uap3-verb
20. Share content from your app (desktop apps): https://learn.microsoft.com/en-us/windows/apps/develop/windows-integration/integrate-sharesheet-send
21. SHDoDragDrop: https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop
22. DoDragDrop: https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop
23. SHAddToRecentDocs: https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shaddtorecentdocs
