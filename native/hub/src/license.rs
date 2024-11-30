use std::{fs::File, io::Read, path::Path, sync::Arc};

use anyhow::Result;
use rinf::DartSignal;

use database::connection::MainDbConnection;

use crate::{
    RegisterLicenseRequest, RegisterLicenseResponse, ValidateLicenseRequest,
    ValidateLicenseResponse,
};
use sha2::{Digest, Sha256};

#[cfg(target_os = "windows")]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>> {
    use tokio::time::{sleep, Duration};
    use windows::Foundation::AsyncStatus;
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

const LICENSE0: &str = "9f93edc284a718a56f82a2e3cf05a83f07d87d36f7f8e4f826789dde40ab9537";
const LICENSE1: &str = "e8b8db66d3d3e04bcce29bb59d7ca1460e92df89297ffca570e5954cc2a426c6";

pub async fn register_license_request(
    _main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<RegisterLicenseRequest>,
) -> Result<()> {
    let path = dart_signal.message.path;

    // Read the file content
    let file_content = match read_file_content(path).await {
        Ok(content) => content,
        Err(e) => {
            let response = RegisterLicenseResponse {
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

    let mut hasher = Sha256::new();
    hasher.update(license_hash.as_bytes());
    let result = hasher.finalize();
    let validation_hash = format!("{:x}", result);

    // Validate the License
    let valid = validation_hash == LICENSE0 || validation_hash == LICENSE1;

    // Construct the response
    let response = RegisterLicenseResponse {
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
    _main_db: Arc<MainDbConnection>,
    dart_signal: DartSignal<ValidateLicenseRequest>,
) -> Result<()> {
    let license = dart_signal.message.license;

    let mut is_pro = false;
    let mut is_store_mode = false;

    let args: Vec<String> = std::env::args().collect();
    let pro_via_args = args.contains(&"--pro".to_string());
    let store_via_args = args.contains(&"--store".to_string());

    is_pro = is_pro || pro_via_args;
    is_store_mode = is_store_mode || store_via_args;

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
            Some((_, _, is_trial)) => ValidateLicenseResponse {
                is_pro: is_pro || !is_trial,
                is_store_mode: true,
            },
            _ => ValidateLicenseResponse {
                is_pro,
                is_store_mode,
            },
        },
        Err(_) => ValidateLicenseResponse {
            is_pro,
            is_store_mode,
        },
    };

    response.send_signal_to_dart();

    Ok(())
}
