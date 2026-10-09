# Shortcut icon verification

These are the icons Windows Shell resolved from the saved broken Atlas pin and
the same pin after changing its icon source to the installed executable.
Captured on October 9, 2026. They are icon extracts, not taskbar screenshots.

| Previous MSI icon path no longer exists | Stable executable icon |
| --- | --- |
| ![Generic shortcut icon](before-icon.png) | ![Atlas shortcut icon](after-icon.png) |

The 6.0.5 MSI leaves the Start-menu and desktop shortcuts' `Icon_` field unset,
so Windows uses their target executable's embedded icon. Both shortcuts retain
their target and `System.AppUserModel.ID`. The MSI still includes `ProductIcon`
for Windows Installed Apps.

An isolated shortcut with no explicit icon source resolved the same underlying
Shell icon as its executable, both before and after replacing the 6.0.4
executable with the locally built 6.0.5 executable. The shortcut arrow overlay is
expected. No installer was run against the live installation for this check.
