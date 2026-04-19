; Symbiont DSL highlight queries for tree-sitter

; Keywords — definition forms
"agent" @keyword
"policy" @keyword
"function" @keyword
"type" @keyword
"schedule" @keyword
"channel" @keyword
"memory" @keyword
"webhook" @keyword
"metadata" @keyword
"capabilities" @keyword
"with" @keyword
"search" @keyword
"filter" @keyword
"data_classification" @keyword

; Control flow
"if" @keyword
"else" @keyword
"let" @keyword
"return" @keyword

; Policy actions
"allow" @keyword.operator
"deny" @keyword.operator
"require" @keyword.operator
"audit" @keyword.operator

; Type names
"String" @type.builtin
"int" @type.builtin
"float" @type.builtin
"bool" @type.builtin

; Booleans
"true" @constant.builtin
"false" @constant.builtin

; Literals
(string) @string
(number) @number
(duration_literal) @number
(boolean) @constant.builtin

; Identifiers in definition positions
(agent_definition (identifier) @function.definition)
(policy_definition (identifier) @function.definition)
(function_definition (identifier) @function.definition)
(type_definition (identifier) @type.definition)
(schedule_definition (identifier) @function.definition)
(channel_definition (identifier) @function.definition)
(memory_definition (identifier) @function.definition)
(webhook_definition (identifier) @function.definition)

; Function calls
(function_call (identifier) @function.call)

; Parameters
(parameter (identifier) @variable.parameter)
(parameter (type) @type)

; Named arguments
(named_argument (identifier) @property)

; Metadata keys
(metadata_pair (identifier) @property)

; Fields in struct types
(field (identifier) @property)
(field (type) @type)

; Schedule/channel/memory/webhook properties
(schedule_property (identifier) @property)
(channel_property (identifier) @property)
(memory_property (identifier) @property)
(webhook_property (identifier) @property)

; With block attributes
(with_attribute (identifier) @property)

; Comments
(comment) @comment

; Punctuation
"{" @punctuation.bracket
"}" @punctuation.bracket
"(" @punctuation.bracket
")" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
":" @punctuation.delimiter
"," @punctuation.delimiter
";" @punctuation.delimiter
"=" @operator
"->" @operator
