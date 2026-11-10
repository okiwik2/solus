//! Windows user verification (Windows Hello face / fingerprint / PIN) before revealing secrets.

use windows::core::HSTRING;
use windows_future::{AsyncStatus, IAsyncOperation};
use windows::Security::Credentials::UI::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::Win32::UI::WindowsAndMessaging::*;

fn pump_until(op: &IAsyncOperation<UserConsentVerificationResult>) {
    unsafe {
        let mut msg = MSG::default();
        loop {
            match op.Status() {
                Ok(AsyncStatus::Started) => {}
                _ => return,
            }
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
}

/// Returns true only when Windows confirms the user. Without Windows Hello configured we refuse,
/// because there is no other supported way for an app to re-check the account password.
pub fn verify(hwnd: Option<HWND>, reason: &str) -> bool {
    let available = UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|op| op.join())
        .map(|a| a == UserConsentVerifierAvailability::Available)
        .unwrap_or(false);
    if !available {
        let msg = HSTRING::from(
            "To view or export passwords, Solus asks Windows to confirm it's you.\n\nSet up Windows Hello (a PIN, fingerprint or face) in Settings > Accounts > Sign-in options, then try again.",
        );
        unsafe {
            MessageBoxW(hwnd.unwrap_or_default().into(), &msg, &HSTRING::from("Solus"), MB_OK | MB_ICONINFORMATION);
        }
        return false;
    }
    let r = (|| -> windows::core::Result<bool> {
        let interop = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
        let op: IAsyncOperation<UserConsentVerificationResult> =
            unsafe { interop.RequestVerificationForWindowAsync(hwnd.unwrap_or_default(), &HSTRING::from(reason))? };
        pump_until(&op);
        Ok(op.GetResults()? == UserConsentVerificationResult::Verified)
    })();
    match r {
        Ok(v) => v,
        Err(e) => {
            logf!("user verification failed: {e}");
            false
        }
    }
}
