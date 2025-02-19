use once_cell::sync::OnceCell;
use std::path::PathBuf;

static CONFIG_DIR: OnceCell<PathBuf> = OnceCell::new();

pub fn init_config_dir(path: PathBuf) {
    CONFIG_DIR.set(path).expect("Config dir already set");
}

pub fn get_binary_path(name: &str) -> PathBuf {
    CONFIG_DIR
        .get()
        .expect("Config dir not initialized")
        .join(if cfg!(windows) {
            format!("{}.exe", name)
        } else {
            name.to_string()
        })
}
