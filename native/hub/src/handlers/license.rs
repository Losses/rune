use std::{fs::File, io::Read, path::Path, sync::Arc};

use anyhow::Result;

use database::connection::MainDbConnection;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};
use sha2::{Digest, Sha256};

#[cfg(target_os = "windows")]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>> {
    use tokio::time::{Duration, sleep};
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

#[cfg(target_os = "macos")]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>, &'static str> {
    use crate::apple_bridge::get_bundle_id;
    let bundle_id = get_bundle_id();
    if bundle_id == "ci.not.rune.appstore" {
        let is_active = true;
        let is_trial = false;
        Ok(Some((bundle_id, is_active, is_trial)))
    } else {
        Ok(None)
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub async fn check_store_license() -> Result<Option<(String, bool, bool)>, &'static str> {
    Ok(None)
}

const LICENSES: [&str; 18] = [
    "9f93edc284a718a56f82a2e3cf05a83f07d87d36f7f8e4f826789dde40ab9537",
    "e8b8db66d3d3e04bcce29bb59d7ca1460e92df89297ffca570e5954cc2a426c6",
    // AIFF (7)
    "45f3afb107d25658939577ec6bbafae81f4e5c3101d9c134b306480514bdf765",
    "a7347684490d0e38515bdbdcde99aeef36de150625ea9199949732910fef883e",
    // WAV (6)
    "efd49dd984ec9ecc8a98c99f1fd5f0381295a8d2e7badcafa835cfe2a3629305",
    "0b05a560faa60e7f31cf54cd624a706fd097a6bcf57cda2153acdb581c0bcf9c",
    // M4A (5)
    "b21d8de0428469da2a1b1b5ba2ec9e2dcdc9c4123a24d829f1a208fc9b0f1fe5",
    "cd82bdfbe46e401dd07374ef84628a4a9dbdc2958dd7d38bdf9a51f19e95a5cb",
    // OGG (4)
    "e1f29c7c8379e0bc47f618e19a003fed9c4482c81ddc8586857c2ef427da492e",
    "6daacc6f6afc9f11534cded4a73d2a5389932aab643000846dd20eb9b152be7d",
    // M4A (3)
    "55af938a5d704b6e45e06b7ecbbb995a0f8780fa74d376fc41ed07ad5ca36da5",
    "5af78fea7f52f95c828c6032866440d8320f321add123f8325733552176b1659",
    // FLAC (2)
    "b9e2a7074af56ad09f9a1f0cfa3b826691ee77c8a45177fd3ddfc6c89e5a753d",
    "d5fd1257a42c08d9dbdce5f584add0bcab9232396b6622877d07e63817b35273",
    // MP3 (1)
    "ce0e14233bc4529f8b65447be04867a5df1cfe6807510e4d6c9d669cb916106a",
    "44312b0643aa4a787c74975a32f15f5c4a9d028ed4ca340646634defdac7963d",
    // MP3 (0)
    "400c8c468cde5ee301dc92a4fb76ff2cecff2050d8f28e37ae1d8e7ecfc0aee4",
    "a3756e26611e76424d262bced05b2d4c0aa73e411b7f65d8347154eecde5ab85",
];

impl ParamsExtractor for RegisterLicenseRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for RegisterLicenseRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = RegisterLicenseResponse;

    async fn handle(
        &self,
        (_main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let path = &dart_signal.path;

        // Read the file content
        let file_content = match read_file_content(path).await {
            Ok(content) => content,
            Err(e) => {
                return Ok(Some(RegisterLicenseResponse {
                    valid: false,
                    license: None,
                    success: false,
                    error: Some(format!("{e:#?}")),
                }));
            }
        };

        // Compute the SHA256 hash of the file content
        let mut hasher = Sha256::new();
        hasher.update(&file_content);
        let result = hasher.finalize();
        let license_hash = format!("{result:x}");

        let mut hasher = Sha256::new();
        hasher.update(license_hash.as_bytes());
        let result = hasher.finalize();
        let validation_hash = format!("{result:x}");

        // Validate the License
        let valid = LICENSES.contains(&validation_hash.as_str());

        Ok(Some(RegisterLicenseResponse {
            valid,
            license: Some(license_hash),
            success: true,
            error: None,
        }))
    }
}

// Helper function: Asynchronously read file content
async fn read_file_content<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

impl ParamsExtractor for ValidateLicenseRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for ValidateLicenseRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = ValidateLicenseResponse;

    async fn handle(
        &self,
        (_main_db,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let license = &dart_signal.license;

        let mut is_pro = false;
        let mut is_store_mode = false;

        let args: Vec<String> = std::env::args().collect();
        let pro_via_args = args.contains(&"--pro".to_string());
        let store_via_args = args.contains(&"--store".to_string());

        is_pro = is_pro || pro_via_args;
        is_store_mode = is_store_mode || store_via_args;

        if !is_pro && let Some(license) = license {
            let mut hasher = Sha256::new();
            hasher.update(license.as_bytes());
            let result = hasher.finalize();
            let hash_str = format!("{result:x}");

            let pro_via_license = LICENSES.contains(&hash_str.as_str());

            is_pro = is_pro || pro_via_license;
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

        Ok(Some(response))
    }
}
