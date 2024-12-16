pub mod colors;
pub mod configuration;
pub mod paths;

pub use colors::Theme;
pub use configuration::Config;
pub use paths::Paths;
use serde::{Deserialize, Serialize};

use crate::tui::UnitStr;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Switch {
	Always,
	Never,
	Auto,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum OptType {
	Bool(bool),
	Int(u8),
	Int64(u64),
	Switch(Switch),
	UnitStr(UnitStr),
	// Strings have to be last in the enum
	// as almost anything will match them
	String(String),
	VecString(Vec<String>),
}
