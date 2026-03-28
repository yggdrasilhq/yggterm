use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub struct AppIconAssets {
    pub svg_bytes: &'static [u8],
    pub png_512_bytes: &'static [u8],
}

pub struct LinuxInstalledIconSet {
    pub direct_png_path: PathBuf,
    pub direct_svg_path: PathBuf,
}

pub const YGGTERM_ICON_ASSETS: AppIconAssets = AppIconAssets {
    svg_bytes: include_bytes!("../../../assets/brand/yggterm-icon.svg"),
    png_512_bytes: include_bytes!("../../../assets/brand/yggterm-icon-512.png"),
};

pub fn install_linux_icon_assets(
    data_dir: &Path,
    direct_assets_dir: &Path,
    icon_names: &[&str],
    assets: AppIconAssets,
) -> Result<LinuxInstalledIconSet> {
    let pixmaps_dir = data_dir.join("pixmaps");
    let icons_dir = data_dir
        .join("icons")
        .join("hicolor")
        .join("512x512")
        .join("apps");
    let scalable_icons_dir = data_dir
        .join("icons")
        .join("hicolor")
        .join("scalable")
        .join("apps");
    fs::create_dir_all(&pixmaps_dir)?;
    fs::create_dir_all(direct_assets_dir)?;
    fs::create_dir_all(&icons_dir)?;
    fs::create_dir_all(&scalable_icons_dir)?;

    for icon_name in icon_names {
        let icon_path = icons_dir.join(format!("{icon_name}.png"));
        write_if_changed(&icon_path, assets.png_512_bytes)?;
        let scalable_icon_path = scalable_icons_dir.join(format!("{icon_name}.svg"));
        write_if_changed(&scalable_icon_path, assets.svg_bytes)?;
    }

    let pixmaps_icon_path = pixmaps_dir.join("yggterm.png");
    write_if_changed(&pixmaps_icon_path, assets.png_512_bytes)?;
    let pixmaps_scalable_icon_path = pixmaps_dir.join("yggterm.svg");
    write_if_changed(&pixmaps_scalable_icon_path, assets.svg_bytes)?;

    let direct_png_path = direct_assets_dir.join("yggterm.png");
    write_if_changed(&direct_png_path, assets.png_512_bytes)?;
    let direct_svg_path = direct_assets_dir.join("yggterm.svg");
    write_if_changed(&direct_svg_path, assets.svg_bytes)?;

    Ok(LinuxInstalledIconSet {
        direct_png_path,
        direct_svg_path,
    })
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path)
        && existing == bytes
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
