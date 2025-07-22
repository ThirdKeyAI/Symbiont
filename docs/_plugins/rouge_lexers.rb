require 'rouge'

module Rouge
  module Lexers
    class Symbiont < RegexLexer
      title "Symbiont DSL"
      desc "Syntax highlighting for Symbiont DSL files"
      tag 'symbiont'
      aliases 'sym'
      filenames '*.dsl'

      state :root do
        rule %r{//.*$}, Comment::Single
        rule %r{"[^"]*"}, Str::Double
        rule %r{\b\d+(?:\.\d+)?\b}, Num
        rule %r{\b(?:true|false)\b}, Keyword::Constant
        rule %r{\b(?:metadata|agent|capabilities|policy|function|type)\b}, Keyword::Declaration
        rule %r{\b(?:let|if|else|return)\b}, Keyword::Reserved
        rule %r{\b(?:allow|deny|require|audit)\b}, Keyword::Namespace
        rule %r{\b(?:String|int|float|bool|Result|Option)\b}, Keyword::Type
        rule %r{->}, Operator
        rule %r{[=:]}, Operator
        rule %r{[(){}\[\],;<>]}, Punctuation
        rule %r{[a-zA-Z_][a-zA-Z0-9_]*}, Name
        rule %r{\s+}, Text
      end
    end
  end
end