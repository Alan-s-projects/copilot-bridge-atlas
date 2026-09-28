use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// Initialize AutoLaunch instance
fn get_auto_launch() -> Result<AutoLaunch, AppError> {
    let app_name = "Copilot Bridge Atlas";
    let exe_path = std::env::current_exe()
        .map_err(|e| AppError::Message(format!("Unable to obtain application path: {e}")))?;

    let auto_launch = AutoLaunchBuilder::new()
        .set_app_name(app_name)
        .set_app_path(&exe_path.to_string_lossy())
        .build()
        .map_err(|e| AppError::Message(format!("Failed to create AutoLaunch: {e}")))?;

    Ok(auto_launch)
}

/// Enable auto-start at boot
pub fn enable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .enable()
        .map_err(|e| AppError::Message(format!("Failed to enable auto-start at boot: {e}")))?;
    log::info!("Autostart enabled");
    Ok(())
}

/// Disable autostart at boot
pub fn disable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .disable()
        .map_err(|e| AppError::Message(format!("Failed to disable auto-start at boot: {e}")))?;
    log::info!("Autostart disabled");
    Ok(())
}
