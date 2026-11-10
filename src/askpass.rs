//! 面板的连接要密码时，把用户在面板上输的密码交给 `ssh`、`sftp`（Windows）。
//!
//! Windows 自带的 OpenSSH 不能连接复用，监控和文件页要另开连接（见 `ssh::Sharing`）。只能输密码的服务器，
//! 就靠 OpenSSH 的 `SSH_ASKPASS`：要密码时它起 `SSH_ASKPASS` 指的程序，把提示当参数传过去，读它打印的一行当密码。
//! 这个程序就是 Nimbus 自己：[`configure`] 给面板的 `ssh`、`sftp` 设上环境变量，Nimbus 被这样起来时
//! （`NIMBUS_ASKPASS=1`）不开窗口，[`main_if_asked`] 回答完就退出。
//!
//! 密码不明文出现在环境变量里：用 Windows 的 DPAPI（`CryptProtectData`）按当前用户加密，只有这个用户能解开。
//! 只回答问密码的提示（`… password:`）：问密钥口令、两步验证码、要不要信这台服务器的，一律不答（退出码 1），
//! `ssh` 就当这一步失败，不会拿密码去当别的东西用。

use std::process::Command;

/// Nimbus 被当成取密码的程序起来时，环境里有这个
const MARKER: &str = "NIMBUS_ASKPASS";
/// 加密过的密码（十六进制）
const SECRET: &str = "NIMBUS_ASKPASS_SECRET";

/// 让 `command`（面板的 `ssh`、`sftp`）要密码时找 Nimbus 要，答 `password`。
/// 起这个程序的路径、加密都要成功；不成功返回 `false`，`command` 原样（它会像以前一样只用密钥）
pub fn configure(command: &mut Command, password: &str) -> bool {
    let (Some(exe), Some(secret)) = (helper(), protect(password.as_bytes())) else {
        return false;
    };
    command
        .env("SSH_ASKPASS", exe)
        // 有没有终端都用它（OpenSSH 8.4 起）；没有图形界面的老规矩要 DISPLAY，设一个占位
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "nimbus:0")
        .env(MARKER, "1")
        .env(SECRET, to_hex(&secret));
    true
}

/// 取密码的程序：Nimbus 自己。测试程序当不了（它的 `main` 是跑测试的），连真服务器的实验室测试用
/// `NIMBUS_TEST_ASKPASS_EXE` 指向编出来的应用
fn helper() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(exe) = std::env::var_os("NIMBUS_TEST_ASKPASS_EXE") {
        return Some(exe.into());
    }
    std::env::current_exe().ok()
}

/// Nimbus 是被 `ssh` 当成取密码的程序起来的：回答这一次，返回退出码（调用的人退出）。不是的话是 `None`，照常起应用
pub fn main_if_asked() -> Option<i32> {
    if std::env::var_os(MARKER).is_none_or(|value| value != "1") {
        return None;
    }
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let password = std::env::var(SECRET)
        .ok()
        .and_then(|hex| from_hex(&hex))
        .and_then(|secret| unprotect(&secret))
        .and_then(|bytes| String::from_utf8(bytes).ok());
    match answer(&prompt, password) {
        Some(password) => {
            use std::io::Write;
            let mut stdout = std::io::stdout();
            let written = writeln!(stdout, "{password}").and_then(|()| stdout.flush());
            Some(if written.is_ok() { 0 } else { 1 })
        }
        None => Some(1),
    }
}

/// `ssh` 的提示 `prompt` 该不该答、答什么：只答问密码的（`user@host's password:`、键盘交互认证里的 `Password:`）。
/// 问密钥口令（passphrase）、两步验证码、要不要信这台服务器的不答
fn answer(prompt: &str, password: Option<String>) -> Option<String> {
    let lower = prompt.to_lowercase();
    let asks_password = lower.contains("password") || prompt.contains("密码");
    if !asks_password || lower.contains("passphrase") {
        return None;
    }
    password
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// DPAPI 加密：只有当前用户（在这台机器上）解得开
#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi::run(data, true)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi::run(data, false)
}

/// 别的平台用不到（面板借终端的连接，不用另外认证）
#[cfg(not(windows))]
fn protect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    /// 加密（`protect`）或者解密，失败是 `None`
    pub fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).ok()?,
            pbData: data.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: 输入指向 `data`，调用期间有效、不会被改；结果由系统分配，下面拷出来以后用 LocalFree 放掉
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            }
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: 成功时 `pbData` 指向 `cbData` 个字节
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        // SAFETY: 系统用 LocalAlloc 分配的，只放这一次
        unsafe { LocalFree(output.pbData as HLOCAL) };
        Some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只答问密码的提示；问口令、验证码、要不要信服务器的不答；没有密码就答不了
    #[test]
    fn only_password_prompts_are_answered() {
        let password = || Some("s3cret".to_string());
        for prompt in [
            "deploy@prod-hk's password: ",
            "Password: ",
            "(deploy@10.0.0.2) Password: ",
            "密码：",
        ] {
            assert_eq!(
                answer(prompt, password()).as_deref(),
                Some("s3cret"),
                "{prompt}"
            );
        }
        for prompt in [
            "Enter passphrase for key 'C:\\Users\\me/.ssh/id_ed25519': ",
            "Verification code: ",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            "",
        ] {
            assert_eq!(answer(prompt, password()), None, "{prompt}");
        }
        assert_eq!(answer("Password: ", None), None);
    }

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(from_hex(&to_hex(&bytes)).as_deref(), Some(&bytes[..]));
        assert_eq!(from_hex("abc"), None, "奇数位");
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex(""), Some(Vec::new()));
    }

    /// DPAPI：加密以后不是原文，解得回来；随便一串解不开
    #[cfg(windows)]
    #[test]
    fn secrets_are_protected_for_this_user() {
        let secret = protect("密码 p@ss".as_bytes()).expect("加得了密");
        assert!(!secret.windows(4).any(|window| window == b"p@ss"));
        assert_eq!(unprotect(&secret).as_deref(), Some("密码 p@ss".as_bytes()));
        assert_eq!(unprotect(b"not a blob"), None);
    }

    /// 设上的环境变量：起谁、什么时候都用它、加密过的密码（不是原文）
    #[cfg(windows)]
    #[test]
    fn commands_ask_nimbus_for_the_password() {
        let mut command = Command::new("ssh");
        assert!(configure(&mut command, "hunter2"));
        let environment: std::collections::HashMap<String, String> = command
            .get_envs()
            .filter_map(|(name, value)| {
                Some((
                    name.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert_eq!(environment["SSH_ASKPASS_REQUIRE"], "force");
        assert_eq!(environment[MARKER], "1");
        assert_eq!(
            environment["SSH_ASKPASS"],
            std::env::current_exe().expect("有").to_string_lossy()
        );
        let secret = &environment[SECRET];
        assert!(!secret.contains("hunter2"));
        let decrypted = from_hex(secret).and_then(|bytes| unprotect(&bytes));
        assert_eq!(decrypted.as_deref(), Some(&b"hunter2"[..]));
    }
}
