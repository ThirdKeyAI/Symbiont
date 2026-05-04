const PREC = {
  or: 1,
  and: 2,
  equality: 3,
  comparison: 4,
  additive: 5,
  multiplicative: 6,
  unary: 7,
  postfix: 8,
  primary: 9,
  type_literal: 10,
};

module.exports = grammar({
  name: 'symbiont',

  extras: $ => [
    /\s+/,
    $.comment,
  ],

  word: $ => $.identifier,

  conflicts: $ => [
    [$.record, $.block],
    [$.if_expression, $.if_statement],
    [$.type_literal, $.value],
    [$.record, $.match_statement],
  ],

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
      $.channel_definition,
      $.memory_definition,
      $.webhook_definition,
    ),

    // ============================================================
    // Top-level definition forms
    // ============================================================

    metadata_block: $ => seq('metadata', '{', repeat($.metadata_pair), '}'),

    metadata_pair: $ => seq(
      $.identifier,
      choice('=', ':'),
      choice($.value, $.array, $.record),
      optional(','),
    ),

    agent_definition: $ => seq(
      'agent',
      $.identifier,
      optional(seq('(', repeat(seq($.parameter, optional(','))), ')')),
      optional(seq('->', $.type)),
      '{',
      repeat($._agent_item),
      '}',
    ),

    _agent_item: $ => choice(
      $.capabilities_declaration,
      $.policy_definition,
      $.function_definition,
      $.with_block,
      $.comment,
    ),

    capabilities_declaration: $ => seq(
      'capabilities',
      choice('=', ':'),
      $.array,
    ),

    policy_definition: $ => seq(
      'policy',
      $.identifier,
      '{',
      repeat($.policy_rule),
      '}',
    ),

    policy_rule: $ => seq(
      choice('allow', 'deny', 'require', 'audit'),
      ':',
      $.expression,
      optional(seq('if', $.expression)),
    ),

    function_definition: $ => seq(
      'function',
      $.identifier,
      '(',
      repeat(seq($.parameter, optional(','))),
      ')',
      optional(seq('->', $.type)),
      $.block,
    ),

    schedule_definition: $ => seq(
      'schedule',
      $.identifier,
      '{',
      repeat($.schedule_property),
      '}',
    ),

    schedule_property: $ => seq(
      $.identifier,
      choice('=', ':'),
      $.value,
      optional(','),
    ),

    channel_definition: $ => seq(
      'channel',
      $.identifier,
      '{',
      repeat(choice(
        $.channel_property,
        $.channel_policy_block,
        $.channel_data_classification_block,
      )),
      '}',
    ),

    channel_property: $ => seq(
      $.identifier,
      choice('=', ':'),
      choice($.value, $.array),
      optional(','),
    ),

    channel_policy_block: $ => seq(
      'policy',
      $.identifier,
      '{',
      repeat($.policy_rule),
      '}',
    ),

    channel_data_classification_block: $ => seq(
      'data_classification',
      '{',
      repeat($.data_classification_rule),
      '}',
    ),

    data_classification_rule: $ => seq(
      $.identifier,
      ':',
      $.identifier,
      optional(','),
    ),

    memory_definition: $ => seq(
      'memory',
      $.identifier,
      '{',
      repeat(choice($.memory_property, $.memory_search_block)),
      '}',
    ),

    memory_property: $ => seq(
      $.identifier,
      $.value,
      optional(','),
    ),

    memory_search_block: $ => seq(
      'search',
      '{',
      repeat($.memory_search_property),
      '}',
    ),

    memory_search_property: $ => seq(
      $.identifier,
      $.value,
      optional(','),
    ),

    webhook_definition: $ => seq(
      'webhook',
      $.identifier,
      '{',
      repeat(choice($.webhook_property, $.webhook_filter_block)),
      '}',
    ),

    webhook_property: $ => seq(
      $.identifier,
      $.value,
      optional(','),
    ),

    webhook_filter_block: $ => seq(
      'filter',
      '{',
      repeat($.webhook_filter_property),
      '}',
    ),

    webhook_filter_property: $ => seq(
      $.identifier,
      $.value,
      optional(','),
    ),

    parameter: $ => seq($.identifier, ':', $.type),

    type_definition: $ => seq('type', $.identifier, '=', $.type_spec),

    type_spec: $ => choice($.struct_type, $.identifier),

    struct_type: $ => seq('{', repeat(seq($.field, optional(','))), '}'),

    field: $ => seq($.identifier, ':', $.type),

    // ============================================================
    // Types — supports multi-arg generics: Map<K, V>, Result<T, E>
    // ============================================================

    type: $ => choice(
      'String',
      'int',
      'float',
      'bool',
      seq($.identifier, '<', $.type, repeat(seq(',', $.type)), '>'),
      $.identifier,
    ),

    // ============================================================
    // With-block
    // ============================================================

    with_block: $ => seq(
      'with',
      repeat(seq($.with_attribute, optional(','))),
      $.block,
    ),

    with_attribute: $ => seq(
      $.identifier,
      '=',
      choice($.value, $.array),
    ),

    // ============================================================
    // Statements
    // ============================================================

    block: $ => seq(
      '{',
      repeat($.statement),
      optional($.expression),
      '}',
    ),

    statement: $ => choice(
      $.let_statement,
      $.if_statement,
      $.for_statement,
      $.match_statement,
      $.try_statement,
      $.return_statement,
      $.assignment_statement,
      $.expression_statement,
      $.comment,
    ),

    let_statement: $ => seq(
      'let',
      $.identifier,
      '=',
      $.expression,
      ';',
    ),

    if_statement: $ => prec.right(seq(
      'if',
      choice(
        $.expression,
        seq('let', $._pattern, '=', $.expression),
      ),
      $.block,
      optional(seq('else', choice($.if_statement, $.block))),
    )),

    _pattern: $ => choice(
      $.identifier,
      seq($.identifier, '(', repeat(seq($._pattern, optional(','))), ')'),
      '_',
    ),

    for_statement: $ => seq('for', $.identifier, 'in', $.expression, $.block),

    match_statement: $ => seq(
      'match',
      $.expression,
      '{',
      repeat($.match_arm),
      '}',
    ),

    match_arm: $ => seq(
      $._match_pattern,
      '=>',
      choice(seq($.expression, optional(',')), $.block, seq($.block, optional(','))),
    ),

    _match_pattern: $ => choice(
      $.string,
      $.number,
      $.boolean,
      '_',
      $.identifier,
    ),

    try_statement: $ => seq('try', $.block, repeat($.catch_clause)),

    catch_clause: $ => seq(
      'catch',
      '(',
      $.identifier,
      optional($.identifier),
      ')',
      $.block,
    ),

    return_statement: $ => seq(
      'return',
      optional($.expression),
      ';',
    ),

    assignment_statement: $ => seq(
      $._postfix_expression,
      choice('=', '+=', '-=', '*=', '/=', '%='),
      $.expression,
      ';',
    ),

    expression_statement: $ => seq($.expression, ';'),

    // ============================================================
    // Expressions — precedence ladder
    // ============================================================

    expression: $ => $._or_expr,

    _or_expr: $ => choice(
      prec.left(PREC.or, seq($._or_expr, '||', $._and_expr)),
      $._and_expr,
    ),

    _and_expr: $ => choice(
      prec.left(PREC.and, seq($._and_expr, '&&', $._equality_expr)),
      $._equality_expr,
    ),

    _equality_expr: $ => choice(
      prec.left(PREC.equality, seq(
        $._equality_expr,
        choice('==', '!=', 'in'),
        $._comparison_expr,
      )),
      $._comparison_expr,
    ),

    _comparison_expr: $ => choice(
      prec.left(PREC.comparison, seq(
        $._comparison_expr,
        choice('<', '>', '<=', '>='),
        $._additive_expr,
      )),
      $._additive_expr,
    ),

    _additive_expr: $ => choice(
      prec.left(PREC.additive, seq(
        $._additive_expr,
        choice('+', '-'),
        $._multiplicative_expr,
      )),
      $._multiplicative_expr,
    ),

    _multiplicative_expr: $ => choice(
      prec.left(PREC.multiplicative, seq(
        $._multiplicative_expr,
        choice('*', '/', '%'),
        $._unary_expr,
      )),
      $._unary_expr,
    ),

    _unary_expr: $ => choice(
      prec(PREC.unary, seq('!', $._unary_expr)),
      prec(PREC.unary, seq('not', $._unary_expr)),
      prec(PREC.unary, seq('-', $._unary_expr)),
      $._postfix_expression,
    ),

    _postfix_expression: $ => choice(
      $.member_expression,
      $.call_expression,
      $.index_expression,
      $.type_literal,
      $._primary_expression,
    ),

    member_expression: $ => prec.left(PREC.postfix, seq(
      $._postfix_expression,
      '.',
      $.identifier,
    )),

    call_expression: $ => prec.left(PREC.postfix, seq(
      $._postfix_expression,
      '(',
      optional(seq(
        choice($.expression, $.named_argument),
        repeat(seq(',', choice($.expression, $.named_argument))),
        optional(','),
      )),
      ')',
    )),

    index_expression: $ => prec.left(PREC.postfix, seq(
      $._postfix_expression,
      '[',
      $.expression,
      ']',
    )),

    type_literal: $ => prec.dynamic(-1, seq(
      $.identifier,
      '{',
      repeat(seq($.record_field, optional(','))),
      '}',
    )),

    _primary_expression: $ => choice(
      $.lambda,
      $.if_expression,
      $.value,
      $.vault_url,
      $.array,
      $.record,
      seq('(', $.expression, ')'),
    ),

    if_expression: $ => prec.right(seq(
      'if',
      $.expression,
      $.block,
      'else',
      choice($.if_expression, $.block),
    )),

    lambda: $ => prec.right(seq(
      $.identifier,
      '=>',
      $.expression,
    )),

    record: $ => prec.dynamic(-1, seq(
      '{',
      repeat(seq($.record_field, optional(','))),
      '}',
    )),

    record_field: $ => seq(
      choice($.identifier, $.string),
      ':',
      $.expression,
    ),

    named_argument: $ => seq(
      $.identifier,
      choice('=', ':'),
      $.expression,
    ),

    array: $ => seq(
      '[',
      optional(seq(
        $.expression,
        repeat(seq(',', $.expression)),
        optional(','),
      )),
      ']',
    ),

    value: $ => choice(
      $.string,
      $.duration_literal,
      $.number,
      $.boolean,
      $.identifier,
    ),

    vault_url: $ => /vault:\/\/[A-Za-z0-9_\-\/\.]+/,

    identifier: $ => token(prec(-1, /[a-zA-Z_][a-zA-Z0-9_]*/)),
    string: $ => /"(\\.|[^"\\])*"/,
    duration_literal: $ => /\d+(\.seconds|\.minutes|\.hours|s|m|h|d|w|months|y)/,
    number: $ => /\d+(_\d+)*(\.\d+)?/,
    boolean: $ => choice('true', 'false'),

    comment: $ => token(choice(seq('//', /.*/), seq('#', /.*/))),
  },
});
