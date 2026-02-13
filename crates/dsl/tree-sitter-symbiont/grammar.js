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
      $.function_definition,
      $.schedule_definition,
      $.channel_definition
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
      optional(seq('(', repeat(seq($.parameter, optional(','))), ')')),
      optional(seq('->', $.type)),
      '{',
      repeat($._agent_item),
      '}'
    ),

    _agent_item: $ => choice(
      $.capabilities_declaration,
      $.policy_definition,
      $.function_definition,
      $.with_block
    ),

    with_block: $ => seq(
      'with',
      repeat(seq($.with_attribute, optional(','))),
      $.block
    ),

    with_attribute: $ => seq(
      $.identifier,
      '=',
      $.value
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

    schedule_definition: $ => seq(
      'schedule',
      $.identifier,
      '{',
      repeat($.schedule_property),
      '}'
    ),

    schedule_property: $ => seq(
      $.identifier,
      ':',
      $.value,
      optional(',')
    ),

    channel_definition: $ => seq(
      'channel',
      $.identifier,
      '{',
      repeat(choice(
        $.channel_property,
        $.channel_policy_block,
        $.channel_data_classification_block
      )),
      '}'
    ),

    channel_property: $ => seq(
      $.identifier,
      ':',
      choice($.value, $.array),
      optional(',')
    ),

    channel_policy_block: $ => seq(
      'policy',
      $.identifier,
      '{',
      repeat($.policy_rule),
      '}'
    ),

    channel_data_classification_block: $ => seq(
      'data_classification',
      '{',
      repeat($.data_classification_rule),
      '}'
    ),

    data_classification_rule: $ => seq(
      $.identifier,
      ':',
      $.identifier,
      optional(',')
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
      $.duration_literal,
      $.number,
      $.boolean
    ),

    identifier: $ => token(prec(-1, /[a-zA-Z_][a-zA-Z0-9_]*/)),
    string: $ => /"[^"]*"/,
    duration_literal: $ => /\d+\.(seconds|minutes|hours)/,
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