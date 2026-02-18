#![no_main]
use libfuzzer_sys::fuzz_target;
use repl_core::dsl::{Lexer, Parser};

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Lexer must not panic on any input — only Ok or Err
        let tokens = match Lexer::new(input).tokenize() {
            Ok(tokens) => tokens,
            Err(_) => return,
        };

        // Parser must not panic on any token stream — only Ok or Err
        let _ = Parser::new(tokens).parse();
    }
});
