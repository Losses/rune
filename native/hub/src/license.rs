use std::{fs::File, io::Read, path::Path, sync::Arc};

use anyhow::Result;
use database::connection::MainDbConnection;
use rinf::DartSignal;

use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use tokio::time::{sleep, Duration};
#[cfg(target_os = "windows")]
use windows::Foundation::AsyncStatus;

use crate::{RegisterLibraryResponse, RegisterLicenseRequest, ValidateLibraryResponse, ValidateLicenseRequest};

#[cfg(target_os = "windows")]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>> {
    use windows::Services::Store::StoreContext;

    let context = StoreContext::GetDefault()?;
    let operation = context.GetAppLicenseAsync()?;

    while operation.Status()? != AsyncStatus::Completed {
        sleep(Duration::from_millis(100)).await;
    }

    let license = operation.GetResults()?;

    let sku_store_id = license.SkuStoreId()?.to_string();

    if sku_store_id.is_empty() {
        return Ok(None);
    }

    let is_active = license.IsActive()?;
    let is_trial = license.IsTrial()?;

    Ok(Some((sku_store_id, is_active, is_trial)))
}

#[cfg(not(target_os = "windows"))]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>, &'static str> {
    Ok(None)
}

const LICENSE0: &str = "8774701c931a378e3358517866e4f453068069c4ce7b963e0a9c4e2d0babd378efe53a147b9d62b7f18b318eb985d4b110e3b76b5853ebf083d28eab5426f927";
const LICENSE1: &str = "46e8f0033b5094f3855a5ad460ebf6c797286d2388762984aaf43b9faa2709e6b58597a6999ee84660f173ccc7a9b8733c9711c50152702cdd808c9f8d05c06c";

pub async fn register_license_request(
    _main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RegisterLicenseRequest>,
) -> Result<()> {
    let path = dart_signal.message.path;

    // Read the file content
    let file_content = match read_file_content(path).await {
        Ok(content) => content,
        Err(e) => {
            let response = RegisterLibraryResponse {
                valid: false,
                license: None,
                success: false,
                error: Some(format!("{:#?}", e)),
            };
            response.send_signal_to_dart();
            return Ok(());
        }
    };

    // Compute the SHA256 hash of the file content
    let mut hasher = Sha256::new();
    hasher.update(&file_content);
    let result = hasher.finalize();
    let license_hash = format!("{:x}", result);

    // Validate the License
    let valid = license_hash == LICENSE0 || license_hash == LICENSE1;

    // Construct the response
    let response = RegisterLibraryResponse {
        valid,
        license: Some(license_hash),
        success: true,
        error: None,
    };

    // Send the response
    response.send_signal_to_dart();

    Ok(())
}

// Helper function: Asynchronously read file content
async fn read_file_content<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

pub async fn validate_license_request(
    dart_signal: DartSignal<ValidateLicenseRequest>,
) -> Result<()> {
    let license = dart_signal.message.license;

    let mut is_pro = false;

    let args: Vec<String> = std::env::args().collect();
    let pro_via_args = args.contains(&"--pro".to_string());

    is_pro = is_pro || pro_via_args;

    if !is_pro {
        if let Some(license) = license {
            let mut hasher = Sha256::new();
            hasher.update(license.as_bytes());
            let result = hasher.finalize();
            let hash_str = format!("{:x}", result);

            let pro_via_license = hash_str == LICENSE0 || hash_str == LICENSE1;

            is_pro = is_pro || pro_via_license;
        }
    }

    let store_license = check_store_license().await;

    let response = match store_license {
        Ok(license) => match license {
            Some((_, _, is_trial)) => ValidateLibraryResponse {
                is_pro: is_pro || !is_trial,
                is_store_mode: true,
            },
            _ => ValidateLibraryResponse {
                is_pro,
                is_store_mode: false,
            },
        },
        Err(_) => ValidateLibraryResponse {
            is_pro,
            is_store_mode: false,
        },
    };

    response.send_signal_to_dart();

    Ok(())
}
