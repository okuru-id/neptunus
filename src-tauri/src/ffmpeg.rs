//! The system's `ffmpeg` and `ffprobe` (film remux, extension downloads). An app opened from Finder or the
//! Start menu doesn't get the shell's PATH, so the places package managers put them are looked at too, and
//! on Windows they run without a console window flashing up.

use std::path::PathBuf;
use std::process::Command;

/// `bin` (`"ffmpeg"`, `"ffprobe"`) as a command, from wherever it is installed.
pub fn command(bin: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(find(bin).unwrap_or_else(|| bin.into()));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Where `bin` is: on the PATH, else where Homebrew, MacPorts, winget, Scoop or Chocolatey put it.
fn find(bin: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_owned() };
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let extra: Vec<Option<PathBuf>> = if cfg!(target_os = "macos") {
        vec![Some("/opt/homebrew/bin".into()), Some("/usr/local/bin".into()), Some("/opt/local/bin".into())]
    } else if cfg!(windows) {
        vec![
            // winget's links appear here, even before a new PATH reaches a running app
            env("LOCALAPPDATA").map(|d| d.join(r"Microsoft\WinGet\Links")),
            env("USERPROFILE").map(|d| d.join(r"scoop\shims")),
            env("ProgramData").map(|d| d.join(r"chocolatey\bin")),
            env("ProgramFiles").map(|d| d.join(r"ffmpeg\bin")),
            Some(r"C:\ffmpeg\bin".into()),
        ]
    } else {
        vec![]
    };
    path.into_iter().chain(extra.into_iter().flatten()).map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// `None` when ffmpeg and ffprobe run; else how to install them here.
pub fn missing() -> Option<&'static str> {
    let runs = |b: &str| command(b).arg("-version").output().is_ok_and(|o| o.status.success());
    if runs("ffmpeg") && runs("ffprobe") {
        return None;
    }
    Some(if cfg!(target_os = "macos") {
        "ffmpeg belum terpasang. Pasang lewat Terminal dengan Homebrew: brew install ffmpeg"
    } else if cfg!(windows) {
        "ffmpeg belum terpasang. Pasang lewat PowerShell: winget install Gyan.FFmpeg"
    } else {
        "ffmpeg belum terpasang. Pasang lewat pengelola paket, misalnya: sudo apt install ffmpeg"
    })
}
