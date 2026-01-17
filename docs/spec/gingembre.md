# Gingembre Template Engine Specification

## Introduction

Gingembre is a Jinja-like template engine for the dodeca static site generator. It provides
template inheritance, control flow, expression evaluation, and extensibility through filters
and tests.

This specification defines the syntax and semantics of the template language. It serves as
both documentation and the source of truth for implementation correctness via tracey coverage.

---

# Lexical Structure

## Delimiters

Templates contain raw text interspersed with three types of delimited regions:

> r[delim.expression]
> Expression interpolation MUST use `{{` to open and `}}` to close. The expression
> result is converted to a string and inserted into the output.
>
> ```jinja
> Hello, {{ name }}!
> ```

> r[delim.statement]
> Statement tags MUST use `{%` to open and `%}` to close. Statements control flow
> but do not produce output directly.
>
> ```jinja
> {% if show_greeting %}Hello!{% endif %}
> ```

> r[delim.comment]
> Comments MUST use `{#` to open and `#}` to close. Comment contents are discarded
> and produce no output.
>
> ```jinja
> {# This is a comment #}
> ```

## Whitespace

> r[whitespace.raw-text]
> Text outside delimiters MUST be passed through unchanged, including all whitespace.

> r[whitespace.inside-delimiters]
> Whitespace inside expression and statement delimiters MUST be ignored for parsing
> purposes. `{{ name }}` and `{{name}}` are equivalent.

## Identifiers

> r[ident.syntax]
> Identifiers MUST start with a letter (a-z, A-Z) or underscore, followed by zero or
> more letters, digits, or underscores.

> r[ident.case-sensitive]
> Identifiers MUST be case-sensitive. `Name` and `name` are different identifiers.

## Keywords

> r[keyword.reserved]
> The following words are reserved keywords and MUST NOT be used as identifiers:
> `if`, `elif`, `else`, `endif`, `for`, `in`, `endfor`, `block`, `endblock`,
> `extends`, `include`, `import`, `macro`, `endmacro`, `true`, `True`, `false`,
> `False`, `none`, `None`, `not`, `and`, `or`, `is`, `as`, `set`, `continue`, `break`.

---

# Literals

> r[literal.string]
> String literals MUST be enclosed in single quotes (`'`) or double quotes (`"`).
> The enclosing quote type MUST NOT appear unescaped within the string.
>
> ```jinja
> {{ "hello" }}
> {{ 'world' }}
> ```

> r[literal.integer]
> Integer literals MUST be sequences of decimal digits, optionally preceded by
> a minus sign for negative values.
>
> ```jinja
> {{ 42 }}
> {{ -7 }}
> ```

> r[literal.float]
> Float literals MUST contain a decimal point with digits on both sides.
>
> ```jinja
> {{ 3.14 }}
> {{ -0.5 }}
> ```

> r[literal.boolean]
> Boolean literals MUST be `true`, `True`, `false`, or `False`.
>
> ```jinja
> {{ true }}
> {{ False }}
> ```

> r[literal.none]
> The null value MUST be written as `none` or `None`.
>
> ```jinja
> {{ none }}
> ```

> r[literal.list]
> List literals MUST use square brackets with comma-separated elements.
>
> ```jinja
> {{ [1, 2, 3] }}
> {{ ["a", "b"] }}
> ```

> r[literal.dict]
> Dictionary literals MUST use curly braces with colon-separated key-value pairs.
>
> ```jinja
> {{ {"name": "Alice", "age": 30} }}
> ```

---

# Expressions

## Variables

> r[expr.var.lookup]
> A bare identifier MUST be looked up in the current context, searching from
> innermost scope outward.
>
> ```jinja
> {{ username }}
> ```

> r[expr.var.undefined]
> Accessing an undefined variable MUST NOT raise an error during rendering.
> The result MUST be the null value.

## Field Access

> r[expr.field.dot]
> Field access MUST use dot notation: `expr.field`.
>
> ```jinja
> {{ user.name }}
> {{ page.meta.title }}
> ```

> r[expr.field.missing]
> Accessing a missing field on an object MUST return null, not raise an error.

## Index Access

> r[expr.index.bracket]
> Index access MUST use bracket notation: `expr[index]`.
>
> ```jinja
> {{ items[0] }}
> {{ data["key"] }}
> ```

> r[expr.index.out-of-bounds]
> Accessing an out-of-bounds index on a list MUST return null.

> r[expr.index.missing-key]
> Accessing a missing key on a dictionary MUST return null.

## Operators

### Arithmetic

> r[expr.op.add]
> The `+` operator MUST perform addition on numbers.

> r[expr.op.sub]
> The `-` operator MUST perform subtraction on numbers.

> r[expr.op.mul]
> The `*` operator MUST perform multiplication on numbers.

> r[expr.op.div]
> The `/` operator MUST perform division on numbers, returning a float.

> r[expr.op.floordiv]
> The `//` operator MUST perform floor division, returning an integer.

> r[expr.op.mod]
> The `%` operator MUST compute the modulo (remainder) of division.

> r[expr.op.pow]
> The `**` operator MUST compute exponentiation.

### Comparison

> r[expr.op.eq]
> The `==` operator MUST test equality.

> r[expr.op.ne]
> The `!=` operator MUST test inequality.

> r[expr.op.lt]
> The `<` operator MUST test less-than.

> r[expr.op.le]
> The `<=` operator MUST test less-than-or-equal.

> r[expr.op.gt]
> The `>` operator MUST test greater-than.

> r[expr.op.ge]
> The `>=` operator MUST test greater-than-or-equal.

### Logical

> r[expr.op.and]
> The `and` operator MUST perform logical conjunction, short-circuiting on false.

> r[expr.op.or]
> The `or` operator MUST perform logical disjunction, short-circuiting on true.

> r[expr.op.not]
> The `not` operator MUST perform logical negation.

### Membership

> r[expr.op.in]
> The `in` operator MUST test membership in a list, dict keys, or substring in string.
>
> ```jinja
> {{ "a" in ["a", "b"] }}  {# true #}
> {{ "key" in {"key": 1} }}  {# true #}
> {{ "foo" in "foobar" }}  {# true #}
> ```

> r[expr.op.not-in]
> The `not in` operator MUST test non-membership.

### String Concatenation

> r[expr.op.concat]
> The `~` operator MUST concatenate values as strings.
>
> ```jinja
> {{ "Hello, " ~ name ~ "!" }}
> ```

## Ternary Expression

> r[expr.ternary]
> The ternary expression `value if condition else other` MUST evaluate to `value`
> if `condition` is truthy, otherwise `other`.
>
> ```jinja
> {{ "yes" if enabled else "no" }}
> ```

## Function Calls

> r[expr.call.syntax]
> Function calls MUST use parentheses with comma-separated arguments.
>
> ```jinja
> {{ func(arg1, arg2) }}
> ```

> r[expr.call.kwargs]
> Function calls MAY include keyword arguments using `name=value` syntax.
>
> ```jinja
> {{ func(arg1, key=value) }}
> ```

---

# Filters

Filters transform values using the pipe operator.

> r[filter.syntax]
> Filter application MUST use the pipe operator: `expr | filter`.
>
> ```jinja
> {{ name | upper }}
> ```

> r[filter.chaining]
> Filters MUST be chainable: `expr | filter1 | filter2`.
>
> ```jinja
> {{ name | lower | capitalize }}
> ```

> r[filter.args]
> Filters MAY accept arguments: `expr | filter(arg)` or `expr | filter(key=value)`.
>
> ```jinja
> {{ items | join(", ") }}
> {{ value | default("N/A") }}
> ```

## Built-in Filters

### String Filters

> r[filter.upper]
> The `upper` filter MUST convert a string to uppercase.

> r[filter.lower]
> The `lower` filter MUST convert a string to lowercase.

> r[filter.capitalize]
> The `capitalize` filter MUST capitalize the first character and lowercase the rest.

> r[filter.title]
> The `title` filter MUST capitalize the first character of each word.

> r[filter.trim]
> The `trim` filter MUST remove leading and trailing whitespace.

> r[filter.escape]
> The `escape` filter MUST escape HTML special characters (`<`, `>`, `&`, `"`, `'`).

> r[filter.safe]
> The `safe` filter MUST mark a string as safe, preventing automatic escaping.

### Collection Filters

> r[filter.length]
> The `length` filter MUST return the length of a string, list, or dict.

> r[filter.first]
> The `first` filter MUST return the first element of a list or first character of a string.

> r[filter.last]
> The `last` filter MUST return the last element of a list or last character of a string.

> r[filter.reverse]
> The `reverse` filter MUST reverse a list or string.

> r[filter.sort]
> The `sort` filter MUST sort a list. It MAY accept an `attribute` argument for sorting objects.

> r[filter.join]
> The `join` filter MUST concatenate list elements with a separator string.
>
> ```jinja
> {{ ["a", "b", "c"] | join(", ") }}  {# "a, b, c" #}
> ```

> r[filter.split]
> The `split` filter MUST split a string into a list by a separator.
>
> ```jinja
> {{ "a,b,c" | split(",") }}  {# ["a", "b", "c"] #}
> ```

> r[filter.slice]
> The `slice` filter MUST extract a portion of a list.
>
> ```jinja
> {{ items | slice(0, 3) }}  {# first 3 items #}
> ```

> r[filter.map]
> The `map` filter MUST extract an attribute from each item in a list.
>
> ```jinja
> {{ users | map(attribute="name") }}
> ```

> r[filter.selectattr]
> The `selectattr` filter MUST select items where an attribute passes a test.
>
> ```jinja
> {{ users | selectattr("active", "eq", true) }}
> ```

> r[filter.rejectattr]
> The `rejectattr` filter MUST reject items where an attribute passes a test.

> r[filter.groupby]
> The `groupby` filter MUST group items by an attribute value.
>
> ```jinja
> {{ posts | groupby(attribute="category") }}
> ```

### Utility Filters

> r[filter.default]
> The `default` filter MUST return a fallback value if the input is null or undefined.
>
> ```jinja
> {{ value | default("N/A") }}
> ```

> r[filter.typeof]
> The `typeof` filter MUST return the type name of a value as a string.

### Path Filters

> r[filter.path-segments]
> The `path_segments` filter MUST split a path into segments, removing empty strings.
>
> ```jinja
> {{ "/foo/bar/" | path_segments }}  {# ["foo", "bar"] #}
> ```

> r[filter.path-first]
> The `path_first` filter MUST return the first segment of a path.

> r[filter.path-parent]
> The `path_parent` filter MUST return the parent path.

> r[filter.path-basename]
> The `path_basename` filter MUST return the last segment of a path.

---

# Tests

Tests check properties of values using the `is` operator.

> r[test.syntax]
> Test syntax MUST be `expr is test_name` or `expr is test_name(args)`.
>
> ```jinja
> {% if value is defined %}...{% endif %}
> {% if name is starting_with("Mr.") %}...{% endif %}
> ```

> r[test.negation]
> Tests MAY be negated with `is not`: `expr is not test_name`.
>
> ```jinja
> {% if value is not none %}...{% endif %}
> ```

## Built-in Tests

### Type Tests

> r[test.defined]
> The `defined` test MUST return true if the value is not null/undefined.

> r[test.undefined]
> The `undefined` test MUST return true if the value is null/undefined.

> r[test.none]
> The `none` test MUST return true if the value is null.

> r[test.string]
> The `string` test MUST return true if the value is a string.

> r[test.number]
> The `number` test MUST return true if the value is a number (integer or float).

> r[test.integer]
> The `integer` test MUST return true if the value is an integer.

> r[test.float]
> The `float` test MUST return true if the value is a float (has fractional part).

> r[test.mapping]
> The `mapping` (or `dict`) test MUST return true if the value is a dictionary.

> r[test.iterable]
> The `iterable` (or `sequence`) test MUST return true if the value is iterable.

### Value Tests

> r[test.truthy]
> The `truthy` test MUST return true if the value is truthy.

> r[test.falsy]
> The `falsy` test MUST return true if the value is falsy.

> r[test.empty]
> The `empty` test MUST return true if the value is an empty string, list, or dict.

> r[test.odd]
> The `odd` test MUST return true if the value is an odd integer.

> r[test.even]
> The `even` test MUST return true if the value is an even integer.

### Comparison Tests

> r[test.eq]
> The `eq` (or `equalto`, `sameas`) test MUST return true if values are equal.
>
> ```jinja
> {% if status is eq("active") %}...{% endif %}
> ```

> r[test.ne]
> The `ne` test MUST return true if values are not equal.

> r[test.lt]
> The `lt` (or `lessthan`) test MUST return true if the value is less than the argument.

> r[test.gt]
> The `gt` (or `greaterthan`) test MUST return true if the value is greater than the argument.

### String Tests

> r[test.starting-with]
> The `starting_with` (or `startswith`) test MUST return true if the string starts with the argument.

> r[test.ending-with]
> The `ending_with` (or `endswith`) test MUST return true if the string ends with the argument.

> r[test.containing]
> The `containing` (or `contains`) test MUST return true if the string contains the argument.

---

# Statements

## If Statement

> r[stmt.if.syntax]
> The if statement MUST use the syntax `{% if condition %}body{% endif %}`.
>
> ```jinja
> {% if user %}
>   Hello, {{ user.name }}!
> {% endif %}
> ```

> r[stmt.if.elif]
> The if statement MAY include `{% elif condition %}` branches.
>
> ```jinja
> {% if status == "active" %}
>   Active
> {% elif status == "pending" %}
>   Pending
> {% endif %}
> ```

> r[stmt.if.else]
> The if statement MAY include an `{% else %}` branch.
>
> ```jinja
> {% if items %}
>   {{ items | length }} items
> {% else %}
>   No items
> {% endif %}
> ```

> r[stmt.if.truthiness]
> Condition evaluation MUST use truthiness: null, false, 0, empty string, empty list,
> and empty dict are falsy; all other values are truthy.

## For Loop

> r[stmt.for.syntax]
> The for loop MUST use the syntax `{% for var in iterable %}body{% endfor %}`.
>
> ```jinja
> {% for item in items %}
>   {{ item }}
> {% endfor %}
> ```

> r[stmt.for.tuple-unpacking]
> The for loop MUST support tuple unpacking: `{% for key, value in dict.items() %}`.
>
> ```jinja
> {% for name, score in scores %}
>   {{ name }}: {{ score }}
> {% endfor %}
> ```

> r[stmt.for.else]
> The for loop MAY include an `{% else %}` branch that executes if the iterable is empty.
>
> ```jinja
> {% for item in items %}
>   {{ item }}
> {% else %}
>   No items found.
> {% endfor %}
> ```

> r[stmt.for.loop-var]
> Inside a for loop, a `loop` variable MUST be available with iteration metadata.

> r[stmt.for.loop-index]
> `loop.index` MUST be the 1-based iteration count.

> r[stmt.for.loop-index0]
> `loop.index0` MUST be the 0-based iteration count.

> r[stmt.for.loop-first]
> `loop.first` MUST be true on the first iteration.

> r[stmt.for.loop-last]
> `loop.last` MUST be true on the last iteration.

> r[stmt.for.loop-length]
> `loop.length` MUST be the total number of items in the iterable.

## Continue and Break

> r[stmt.continue]
> The `{% continue %}` statement MUST skip to the next iteration of the enclosing for loop.

> r[stmt.break]
> The `{% break %}` statement MUST exit the enclosing for loop immediately.

## Set Statement

> r[stmt.set.syntax]
> The set statement MUST use the syntax `{% set name = expr %}`.
>
> ```jinja
> {% set greeting = "Hello" %}
> {{ greeting }}
> ```

> r[stmt.set.scope]
> Variables set with `{% set %}` MUST be created in the current scope. They MUST NOT
> modify variables in outer scopes.

---

# Template Inheritance

## Extends

> r[inherit.extends.syntax]
> The extends statement MUST use the syntax `{% extends "path" %}`.
>
> ```jinja
> {% extends "base.html" %}
> ```

> r[inherit.extends.position]
> The extends statement, if present, MUST be the first statement in a template
> (ignoring comments and whitespace).

> r[inherit.extends.single]
> A template MUST NOT extend more than one parent template.

## Block

> r[inherit.block.syntax]
> Block definition MUST use the syntax `{% block name %}content{% endblock %}`.
>
> ```jinja
> {% block title %}Default Title{% endblock %}
> ```

> r[inherit.block.override]
> A child template MAY override parent blocks by defining a block with the same name.
> The child's content replaces the parent's content.

> r[inherit.block.default]
> If a child template does not override a block, the parent's block content MUST be used.

## Include

> r[inherit.include.syntax]
> The include statement MUST use the syntax `{% include "path" %}`.
>
> ```jinja
> {% include "header.html" %}
> ```

> r[inherit.include.context]
> Included templates MUST have access to the current context.

---

# Macros

## Definition

> r[macro.def.syntax]
> Macro definition MUST use the syntax `{% macro name(params) %}body{% endmacro %}`.
>
> ```jinja
> {% macro button(text, type="primary") %}
>   <button class="{{ type }}">{{ text }}</button>
> {% endmacro %}
> ```

> r[macro.def.params]
> Macro parameters MAY have default values using `name=default` syntax.

## Import

> r[macro.import.syntax]
> Macros MUST be imported using `{% import "path" as namespace %}`.
>
> ```jinja
> {% import "macros.html" as m %}
> ```

## Call

> r[macro.call.syntax]
> Macro calls MUST use the syntax `namespace::macro_name(args)`.
>
> ```jinja
> {{ m::button("Click me") }}
> {{ m::button("Submit", type="success") }}
> ```

> r[macro.call.self]
> Macros defined in the current template MUST be called with `self::macro_name(args)`.
>
> ```jinja
> {% macro greet(name) %}Hello, {{ name }}!{% endmacro %}
> {{ self::greet("World") }}
> ```

---

# Scoping

> r[scope.lexical]
> Gingembre MUST use lexical scoping. Inner scopes can read outer variables but
> cannot modify them.

> r[scope.for-loop]
> Each for loop iteration MUST create a new scope. Loop variables and variables
> set within the loop are local to each iteration.

> r[scope.macro]
> Macro bodies MUST execute in their own scope with only the passed parameters
> and global context available.

> r[scope.block]
> Block bodies MUST execute in the context of the template where they are rendered
> (child template context for overridden blocks).

---

# Error Handling

> r[error.span]
> All errors MUST include source location information (offset and length) for
> precise error reporting.

> r[error.undefined-filter]
> Using an undefined filter MUST produce an error.

> r[error.undefined-test]
> Using an undefined test MUST produce an error.

> r[error.syntax]
> Syntax errors (unclosed delimiters, malformed expressions, etc.) MUST be
> reported with the location of the error.

> r[error.type-mismatch]
> Type mismatches in operations (e.g., adding string to number) SHOULD produce
> meaningful error messages.
