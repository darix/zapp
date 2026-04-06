use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;

use zapp_core::device::ids::{is_moonlander_revb, target_name_for_pid};
use zapp_core::device::{self, WatchStatus};
use zapp_core::firmware::{self, Firmware};
use zapp_core::flash::{self, FlashProgress};

#[derive(Parser)]
#[command(name = "zapp", version, about = "⚡ Flash ZSA keyboards")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Flash firmware from a local file
    Flash {
        /// Path to firmware file (.bin or .hex)
        firmware: PathBuf,
    },
    /// Check Oryx for updates and flash if available
    Update,
}

#[derive(Deserialize)]
struct LatestResponse {
    latest: String,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp(None)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Flash { firmware } => cmd_flash(&firmware),
        Commands::Update => cmd_update(),
    }
}

fn cmd_flash(path: &PathBuf) -> Result<()> {
    let fw = firmware::load_firmware(path).context("Failed to load firmware")?;
    print_firmware_info(&fw);
    wait_and_flash(&fw)
}

fn cmd_update() -> Result<()> {
    let connected = device::find_keyboard().context("Failed to scan USB devices")?;
    println!("Found {} connected", connected.keyboard);

    // Parse serial number as "layoutId/revisionId"
    let Some((layout_id, revision_id)) = connected.serial.split_once('/') else {
        bail!(
            "Updates only work if your keyboard is currently flashed with a firmware coming from Oryx."
        );
    };

    if layout_id.is_empty() || revision_id.is_empty() {
        bail!(
            "Updates only work if your keyboard is currently flashed with a firmware coming from Oryx."
        );
    }

    println!("Layout: {layout_id}, revision: {revision_id}");

    // Check for latest revision
    let url = format!("https://oryx.zsa.io/firmware/latest/{layout_id}");
    let resp: LatestResponse = reqwest::blocking::get(&url)
        .and_then(|r| r.error_for_status())
        .context("Failed to check for updates")?
        .json()
        .context("Failed to parse update response")?;

    if revision_id == resp.latest {
        println!("Firmware is already up to date.");
        return Ok(());
    }

    println!(
        "Update available: {} → {}",
        revision_id, resp.latest
    );

    // Download firmware
    let mut download_url = format!("https://oryx.zsa.io/firmware/{}", resp.latest);
    if is_moonlander_revb(connected.pid) {
        download_url.push_str("?alt=true");
    }

    let spinner = new_spinner("Downloading firmware...");

    let fw_bytes = reqwest::blocking::get(&download_url)
        .and_then(|r| r.error_for_status())
        .context("Failed to download firmware")?
        .bytes()
        .context("Failed to read firmware bytes")?;

    spinner.finish_and_clear();

    let fw =
        firmware::load_firmware_from_bytes(&fw_bytes).context("Failed to parse downloaded firmware")?;

    print_firmware_info(&fw);
    wait_and_flash(&fw)
}

fn print_firmware_info(fw: &Firmware) {
    let desc = match fw {
        Firmware::DfuBinary { data, pid, .. } => {
            format!("{} ({} bytes)", target_name_for_pid(*pid), data.len())
        }
        Firmware::IgnitionDual { primary, alternate } => {
            format!(
                "{} + {} ({} + {} bytes)",
                target_name_for_pid(primary.pid),
                target_name_for_pid(alternate.pid),
                primary.data.len(),
                alternate.data.len()
            )
        }
        Firmware::IntelHex { data } => {
            format!("Ergodox EZ ({} bytes)", data.len())
        }
    };
    println!("Firmware loaded: {desc}");
}

fn new_spinner(msg: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["_", "_", "_", "-", "`", "`", "'", "´", "-", "_", "_", "_"])
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message(msg.to_string());
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));
    spinner
}

fn wait_and_flash(fw: &Firmware) -> Result<()> {
    let spinner = new_spinner("Waiting for keyboard in bootloader mode...");

    let device = device::wait_for_bootloader(None, |status| match status {
        WatchStatus::Waiting => {}
        WatchStatus::Found { .. } => {
            spinner.finish_and_clear();
        }
    })
    .context("Failed to detect bootloader")?;

    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("⚡ {bar:40.cyan/blue} {pos}% {msg}")
            .unwrap()
            .progress_chars("█▓░"),
    );

    flash::flash_device(&device, fw, &|progress| match progress {
        FlashProgress::Erasing {
            bytes_erased,
            total_bytes,
        } => {
            let pct = (bytes_erased * 100) / total_bytes;
            pb.set_position(pct as u64);
            pb.set_message("Erasing...");
        }
        FlashProgress::Writing {
            bytes_written,
            total_bytes,
        } => {
            let pct = (bytes_written * 100) / total_bytes;
            pb.set_position(pct as u64);
            pb.set_message("Writing...");
        }
        FlashProgress::Resetting => {
            pb.set_position(100);
            pb.set_message("Resetting...");
        }
        FlashProgress::Complete => {
            pb.finish_with_message("Done!");
        }
    })
    .context("Flash failed")?;

    Ok(())
}
