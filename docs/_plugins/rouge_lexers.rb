require 'rouge'

module Rouge
  module Lexers
    class Symbiont < RegexLexer
      title "Symbiont DSL"
      desc "Syntax highlighting for Symbiont DSL files"
      tag 'symbiont'
      aliases 'sym'
      filenames '*.dsl', '*.symbi'

      state :root do
        rule(/\/\/.*$/, Comment::Single)
        rule(/"[^"]*"/, Literal::String::Double)
        rule(/\b\d+(?:\.\d+)?\b/, Literal::Number)
        rule(/\b(?:true|false)\b/, Literal)
        rule(/\b(?:metadata|agent|capabilities|policy|function|type)\b/, Keyword::Declaration)
        rule(/\b(?:let|if|else|return)\b/, Keyword::Reserved)
        rule(/\b(?:allow|deny|require|audit)\b/, Keyword::Namespace)
        rule(/\b(?:String|int|float|bool|Result|Option)\b/, Keyword::Type)
        rule(/->/, Operator)
        rule(/[=:]/, Operator)
        rule(/[(){}\[\],;<>]/, Punctuation)
        rule(/[a-zA-Z_][a-zA-Z0-9_]*/, Name)
        rule(/\s+/, Text)
      end
    end
  end
end