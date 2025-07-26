module.exports = grammar({
  name: 'symbiont',

  rules: {
    program: $ => repeat($._item),

    _item: $ => choice(
      $.comment,
      $.metadata_block,
      $.agent_definition,
      $.policy_definition,
      $.type_definition,
      $.function_definition
    ),

    metadata_block: $ => seq(
      'metadata',
      '{',
      repeat($.metadata_pair),
      '}'
    ),

    metadata_pair: $ => seq(
      $.identifier,
      ':',
      $.value,
      optional(',')
    ),

    agent_definition: $ => seq(
      'agent',
      $.identifier,
      '{',
      repeat($._agent_item),
      '}'
    ),

    _agent_item: $ => choice(
      $.capabilities_declaration,
      $.policy_definition,
      $.function_definition
    ),

    capabilities_declaration: $ => seq(
      'capabilities',
      ':',
      '[',
      repeat(seq($.identifier, optional(','))),
      ']'
    ),

    policy_definition: $ => seq(
      'policy',
      $.identifier,
      '{',
      repeat($.policy_rule),
      '}'
    ),

    policy_rule: $ => seq(
      choice('allow', 'deny', 'require', 'audit'),
      ':',
      $.expression
    ),

    function_definition: $ => seq(
      'function',
      $.identifier,
      '(',
      repeat(seq($.parameter, optional(','))),
      ')',
      optional(seq('->', $.type)),
      $.block
    ),

    parameter: $ => seq(
      $.identifier,
      ':',
      $.type
    ),

    type_definition: $ => seq(
      'type',
      $.identifier,
      '=',
      $.type_spec
    ),

    type_spec: $ => choice(
      $.struct_type,
      $.identifier
    ),

    struct_type: $ => seq(
      '{',
      repeat(seq($.field, optional(','))),
      '}'
    ),

    field: $ => seq(
      $.identifier,
      ':',
      $.type
    ),

    type: $ => choice(
      'String',
      'int',
      'float',
      'bool',
      seq($.identifier, '<', $.type, '>'),
      $.identifier
    ),

    block: $ => seq(
      '{',
      repeat($.statement),
      '}'
    ),

    statement: $ => choice(
      $.let_statement,
      $.if_statement,
      $.return_statement,
      $.expression_statement
    ),

    let_statement: $ => seq(
      'let',
      $.identifier,
      '=',
      $.expression,
      ';'
    ),

    if_statement: $ => seq(
      'if',
      $.expression,
      $.block,
      optional(seq('else', $.block))
    ),

    return_statement: $ => seq(
      'return',
      $.expression,
      ';'
    ),

    expression_statement: $ => seq(
      $.expression,
      ';'
    ),

    expression: $ => choice(
      $.function_call,
      $.identifier,
      $.value,
      $.array
    ),

    function_call: $ => seq(
      $.identifier,
      '(',
      optional(seq(
        choice($.expression, $.named_argument),
        repeat(seq(',', choice($.expression, $.named_argument)))
      )),
      ')'
    ),

    named_argument: $ => seq(
      $.identifier,
      ':',
      $.expression
    ),

    array: $ => seq(
      '[',
      optional(seq(
        $.expression,
        repeat(seq(',', $.expression))
      )),
      ']'
    ),

    value: $ => choice(
      $.string,
      $.number,
      $.boolean
    ),

    identifier: $ => token(prec(-1, /[a-zA-Z_][a-zA-Z0-9_]*/)),
    string: $ => /"[^"]*"/,
    number: $ => /\d+(\.\d+)?/,
    boolean: $ => choice('true', 'false'),

    comment: $ => token(seq('//', /.*/)),
  },

  extras: $ => [
    /\s+/,
    $.comment,
  ],

  word: $ => $.identifier,

  
});