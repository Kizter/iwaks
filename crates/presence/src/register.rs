//! Register the `discord-<app_id>://` protocol under `HKCU` so Discord
//! treats Iwaks as an app — the registry half of the SDK's `Discord_Register`
//! (the SDK also writes this). Without it Discord will not reliably detect
//! the running process, which the Rich Presence session and the in-app
//! overlay both depend on.
//!
//! Writing under `HKCU` needs no administrator rights; failure is
//! non-fatal (presence still works, overlay detection may not).

use std::io;
use std::path::Path;

use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
};

/// Registry path of the protocol key for `app_id`.
pub fn class_key(app_id: &str) -> String {
    format!(r"Software\Classes\discord-{app_id}")
}

/// The `shell\open\command` value: open the app, passing the URL Discord
/// hands us (join links etc.), with the exe path quoted.
pub fn open_command(exe: &std::path::Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Write the protocol registration. `Ok(())` even when the values already
/// exist; `Err` only on a registry failure.
pub fn register_protocol(app_id: &str, exe: &Path) -> io::Result<()> {
    let root = class_key(app_id);
    // (Default) + URL Protocol marker on the base key.
    set_reg_string(HKEY_CURRENT_USER, &root, None, "")?;
    set_reg_string(HKEY_CURRENT_USER, &root, Some("URL Protocol"), "")?;
    // DefaultIcon → "<exe>,0" so Discord can show an icon for the app.
    set_reg_string(
        HKEY_CURRENT_USER,
        &format!(r"{root}\DefaultIcon"),
        None,
        &format!("{},0", exe.display()),
    )?;
    // open command → launch the app when Discord activates the protocol.
    set_reg_string(
        HKEY_CURRENT_USER,
        &format!(r"{root}\shell\open\command"),
        None,
        &open_command(exe),
    )?;
    Ok(())
}

/// Set one string value under `subkey`. `value_name = None` sets the key's
/// default value (the "(Default)" row).
fn set_reg_string(
    hive: HKEY,
    subkey: &str,
    value_name: Option<&str>,
    data: &str,
) -> io::Result<()> {
    let key_wide = utf16(subkey);
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            hive,
            key_wide.as_ptr(),
            0,
            std::ptr::null(),
            0,   // REG_OPTION_NON_VOLATILE
            0x2, // KEY_SET_VALUE
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let name_wide = value_name.map(utf16);
    let data_wide = utf16(data);
    let status = unsafe {
        RegSetValueExW(
            key,
            name_wide.map_or(std::ptr::null(), |w| w.as_ptr()),
            0,
            1, // REG_SZ
            data_wide.as_ptr() as *const u8,
            (data_wide.len() * 2) as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

/// Nul-terminated UTF-16 encoding of `value`.
fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_key_matches_discord_protocol() {
        assert_eq!(
            class_key("1553676698900365344"),
            r"Software\Classes\discord-1553676698900365344"
        );
    }

    #[test]
    fn open_command_quotes_exe() {
        let exe = Path::new(r"C:\Program Files\Iwaks\iwaks.exe");
        assert_eq!(
            open_command(exe),
            r#""C:\Program Files\Iwaks\iwaks.exe" "%1""#
        );
    }
}
