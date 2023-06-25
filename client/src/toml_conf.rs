pub(crate) use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

const INTERFACE_PREFIX: &str = "cnrs-";
const MAX_CUSTOM_PART_LEN: usize = 8;

#[derive(Debug, Serialize, Deserialize)]
pub struct Addrs {
    pub stun_addr: String,
    pub server_addr: String,
    pub redis_addr: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Interface {
    pub interface_name: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Room {
    pub room_name: String,
    pub username: String,
    pub priv_key: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    pub room: Room,
    pub interface: Interface,
    pub addrs: Addrs,
}

pub static INSTANCE: OnceCell<Config> = OnceCell::new();

impl Config {
    pub fn global() -> &'static Config {
        INSTANCE.get().expect("config is not initialized")
    }

    pub fn generate(
        pattern_path: &str,
        dest_path: &str,
        username: &str,
        room_name: &str,
    ) -> Result<(), anyhow::Error> {
        let pattern_config = std::fs::read_to_string(pattern_path)?;
        let mut conf: Config = toml::from_str(pattern_config.as_str())?;
        let private_key = wireguard_keys::Privkey::generate();
        conf.room.room_name = room_name.to_string();
        conf.room.username = username.to_string();
        conf.room.priv_key = private_key.to_base64();
        conf.interface.interface_name = INTERFACE_PREFIX.to_string() + {
            if room_name.len() <= MAX_CUSTOM_PART_LEN {
                room_name
            } else {
                &room_name[..MAX_CUSTOM_PART_LEN]
            }
        };
        let toml = toml::to_string(&conf)?;
        std::fs::write(dest_path, toml)?;
        INSTANCE.set(conf).expect("Failed to read config file");
        Ok(())
    }

    pub fn from_file(path: &str) -> Result<(), anyhow::Error> {
        let config = std::fs::read_to_string(path)?;
        INSTANCE
            .set(toml::from_str(config.as_str())?)
            .expect("Failed to read config file");
        Ok(())
    }
}
