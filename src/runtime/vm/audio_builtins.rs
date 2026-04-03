use crate::bytecode::Value;
use crate::modules::audio;

use super::Vm;

impl Vm {
    pub(super) fn call_audio_builtin(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        match name {
            "audio_init" => {
                if let Err(err) = self.require_import("audio", name) {
                    return Some(Err(err));
                }
                Some(audio::audio_init_native().map(|_| Value::Null))
            }
            "audio_set_freq" => {
                if let Err(err) = self.require_import("audio", name) {
                    return Some(Err(err));
                }
                if args.len() != 1 {
                    return Some(Err(format!(
                        "audio_set_freq expects 1 arg, got {}",
                        args.len()
                    )));
                }
                match &args[0] {
                    Value::Number(freq) => Some(audio::audio_set_freq_native(*freq).map(|_| Value::Null)),
                    _ => Some(Err("audio_set_freq expects a number".to_string())),
                }
            }
            "audio_set_gain" => {
                if let Err(err) = self.require_import("audio", name) {
                    return Some(Err(err));
                }
                if args.len() != 1 {
                    return Some(Err(format!(
                        "audio_set_gain expects 1 arg, got {}",
                        args.len()
                    )));
                }
                match &args[0] {
                    Value::Number(gain) => Some(audio::audio_set_gain_native(*gain).map(|_| Value::Null)),
                    _ => Some(Err("audio_set_gain expects a number".to_string())),
                }
            }
            "audio_note_on" => {
                if let Err(err) = self.require_import("audio", name) {
                    return Some(Err(err));
                }
                Some(audio::audio_note_on_native().map(|_| Value::Null))
            }
            "audio_note_off" => {
                if let Err(err) = self.require_import("audio", name) {
                    return Some(Err(err));
                }
                Some(audio::audio_note_off_native().map(|_| Value::Null))
            }
            _ => None,
        }
    }
}
