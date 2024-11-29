use anyhow::Result;
use tokio::time::{sleep, Duration};
use windows::Foundation::AsyncStatus;

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
pub async fn check_store_license() -> Result<(bool, bool), &'static str> {
    Ok(None)
}
