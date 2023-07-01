use crate::UIMessage;
use std::fs::OpenOptions;
use std::io::Write;

use once_cell::sync::OnceCell;
use tokio::sync::mpsc::Sender;
// use chrono::Local;

static INSTANCE: OnceCell<Logger> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct Logger {
    table_sender: Option<Sender<UIMessage>>,
    log_path: String,
}

macro_rules! log {
    ($a:expr,$($c:tt)*) => {{
        crate::logger::Logger::global()
            .add_log($a, format!($($c)*).as_str());
    }
    };
}
pub(crate) use log;

#[derive(Debug)]
pub enum LogLevel {
    ERROR,
    FATAL,
    INFO,
    DEBUG,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}


impl Logger {
    pub fn global() -> &'static Logger {
        INSTANCE.get().expect("logger is not initialized")
    }

    pub fn init(path: String, sender: Option<Sender<UIMessage>>) -> Result<(), anyhow::Error> {
        println!("{}", path);
        INSTANCE
            .set(Logger {
                table_sender: sender,
                log_path: path,
            })
            .expect("Failed to initialise logger");

        Ok(())
    }

    pub fn add_log(&self, log_level: LogLevel, msg: &str) {
        let timestamp = chrono::Local::now().format("[%d/%m/%Y %H:%M:%S]");
        let mut log_file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.log_path.as_str())
            .expect("cannot open log file");

        log_file
            .write(format!("{} {:5} - {}\n", timestamp, log_level, msg).as_bytes())
            .expect("write to log file failed");

        match log_level {
            LogLevel::FATAL | LogLevel::ERROR => {
                if let Some(sender) = &self.table_sender {
                    let _ = sender.try_send(UIMessage::ErrorLog(msg.to_string()));
                } else {
                    println!("{}", msg);
                }
                
            }
            LogLevel::INFO => {
                if let Some(sender) = &self.table_sender {
                let _ = sender.try_send(UIMessage::InfoLog(msg.to_string()));
                } else {
                    println!("{}", msg);
                }
            }
            _ => {}
        }
    }
}
