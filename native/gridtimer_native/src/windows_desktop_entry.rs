// v1.1.0.4 - Refresh desktop links only after the release transaction commits.
use std::io;
use std::path::Path;

#[cfg(windows)]
fn desktop_script_command(
    mut command: std::process::Command,
    script: &Path,
    project_root: &Path,
) -> std::process::Command {
    use std::os::windows::process::CommandExt;

    // Windows PowerShell must discover its own modules. A Rust child otherwise
    // inherits pwsh's Core module path, whose Utility module cannot supply the
    // Windows PowerShell Get-FileHash function.
    command.env_remove("PSModulePath");
    let literal = |path: &Path| {
        let raw = path.to_string_lossy();
        let normal = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
        format!("'{}'", normal.replace('\'', "''"))
    };
    let bootstrap = format!(
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); \
         $OutputEncoding=[Console]::OutputEncoding; $ProgressPreference='SilentlyContinue'; \
         $ErrorActionPreference='Stop'; try {{ & {} -ProjectRoot {} -Mode Apply; exit 0 }} \
         catch {{ [Console]::Error.WriteLine($_.ToString()); exit 1 }}",
        literal(script),
        literal(project_root),
    );
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ])
        .arg(bootstrap)
        .creation_flags(0x0800_0000);
    command
}

pub fn maintain_desktop_entry(project_root: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let script = project_root.join("tools/windows_desktop_entry.ps1");
        let output = desktop_script_command(
            std::process::Command::new("powershell.exe"),
            &script,
            project_root,
        )
        .output()?;
        if !output.status.success() {
            return Err(io::Error::new(io::ErrorKind::Other, format!(
                "the Windows release committed, but desktop entry maintenance failed; run tools/windows_desktop_entry.ps1 -Mode Apply after correcting the reported issue: {} {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)
            )));
        }
        println!(
            "Desktop entry updated: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    #[cfg(not(windows))]
    let _ = project_root;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn inherited_core_modules_cannot_hide_hash_command_or_corrupt_utf8_results() {
        let root = std::env::temp_dir().join(format!(
            "TenRateDesktopProcess_{}_{}_'",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let modules = root.join("CoreModules");
        let utility = modules.join("Microsoft.PowerShell.Utility");
        fs::create_dir_all(&utility).unwrap();
        // Reproduce a Core manifest advertising Get-FileHash as a cmdlet while
        // omitting Windows PowerShell's function export. No parent environment
        // changes are needed, so this boundary test can run concurrently.
        fs::write(utility.join("Microsoft.PowerShell.Utility.psd1"),
            "@{ModuleVersion='99.0';GUID='4e924df7-e893-456f-b5b3-bf3b3929420a';CompatiblePSEditions=@('Core');PowerShellVersion='3.0';RootModule='empty.psm1';CmdletsToExport=@('Get-FileHash');FunctionsToExport=@()}"
        ).unwrap();
        fs::write(utility.join("empty.psm1"), "").unwrap();
        fs::write(root.join("payload.bin"), b"desktop-process-boundary").unwrap();
        let inherited_modules = format!(
            "{};{}",
            modules.display(),
            std::env::var("PSModulePath").unwrap_or_default()
        );
        let script = root.join("probe.ps1");
        let write_script = |body: &str| {
            let mut bytes = vec![0xef, 0xbb, 0xbf];
            bytes.extend_from_slice(body.as_bytes());
            fs::write(&script, bytes).unwrap();
        };
        let run = || {
            let mut command = Command::new("powershell.exe");
            command.env("PSModulePath", &inherited_modules);
            desktop_script_command(command, &script, &root)
                .output()
                .unwrap()
        };
        write_script("param($ProjectRoot,$Mode)\n$hash=Get-FileHash -LiteralPath (Join-Path $ProjectRoot 'payload.bin') -Algorithm SHA256\nif($hash.Hash.Length -ne 64 -or $Mode -ne 'Apply'){throw 'invalid process arguments'}\n[Console]::WriteLine('资料🙂'+$hash.Hash)\n[Console]::Error.WriteLine('核验🙂')\n");
        let success = run();
        let stdout = std::str::from_utf8(&success.stdout).unwrap();
        let stderr = std::str::from_utf8(&success.stderr).unwrap();
        assert!(success.status.success(), "{stderr}");
        assert!(stdout.starts_with("资料🙂"), "{stdout}");
        assert!(stderr.contains("核验🙂"), "{stderr}");

        write_script("param($ProjectRoot,$Mode)\nthrow '错误🙂'\n");
        let failure = run();
        assert!(!failure.status.success());
        assert!(std::str::from_utf8(&failure.stderr)
            .unwrap()
            .contains("错误🙂"));
        fs::remove_dir_all(&root).unwrap();
    }
}
