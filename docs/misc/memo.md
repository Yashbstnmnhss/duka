# TODO!:

- dukao
    - package manager
    - build targets
- stdlib
- **LSP**

~~**Codegen**~~

解糖:

- ~~match~~
- ~~object~~
- ~~linq~~

解析:

- ~~object~~
- ~~linq~~
- ~~logic~~

复杂逻辑:

- logic

# Implementing

~~(Lexer) Token~~ →
~~(Parser) AST~~ →
~~Semantic~~ ~~(→ IR)~~ →
~~(Codegen) Instructions~~ →
~~(VM) Runtime~~

## Grammar (lua)

```ebnf
chunk ::= block

block ::= {stat} [retstat]

stat ::=  ‘;’ |
        varlist ‘=’ explist |
        functioncall |
        label |
        break |
        goto Name |
        do block end |
        while exp do block end |
        repeat block until exp |
        if exp then block {elseif exp then block} [else block] end |
        for Name ‘=’ exp ‘,’ exp [‘,’ exp] do block end |
        for namelist in explist do block end |
        function funcname funcbody |
        local function Name funcbody |
        local attnamelist [‘=’ explist]

attnamelist ::=  Name attrib {‘,’ Name attrib}

attrib ::= [‘<’ Name ‘>’]

retstat ::= return [explist] [‘;’]

label ::= ‘::’ Name ‘::’

funcname ::= Name {‘.’ Name} [‘:’ Name]

varlist ::= var {‘,’ var}

var ::=  Name | prefixexp ‘[’ exp ‘]’ | prefixexp ‘.’ Name

namelist ::= Name {‘,’ Name}

explist ::= exp {‘,’ exp}

exp ::=  nil | false | true | Numeral | LiteralString | ‘...’ | functiondef |
        prefixexp | tableconstructor | exp binop exp | unop exp

prefixexp ::= var | functioncall | ‘(’ exp ‘)’

functioncall ::=  prefixexp args | prefixexp ‘:’ Name args

args ::=  ‘(’ [explist] ‘)’ | tableconstructor | LiteralString

functiondef ::= function funcbody

funcbody ::= ‘(’ [parlist] ‘)’ block end

parlist ::= namelist [‘,’ ‘...’] | ‘...’

tableconstructor ::= ‘{’ [fieldlist] ‘}’

fieldlist ::= field {fieldsep field} [fieldsep]

field ::= ‘[’ exp ‘]’ ‘=’ exp | Name ‘=’ exp | exp

fieldsep ::= ‘,’ | ‘;’

binop ::=  ‘+’ | ‘-’ | ‘*’ | ‘/’ | ‘//’ | ‘^’ | ‘%’ |
        ‘&’ | ‘~’ | ‘|’ | ‘>>’ | ‘<<’ | ‘..’ |
        ‘<’ | ‘<=’ | ‘>’ | ‘>=’ | ‘==’ | ‘~=’ |
        and | or

unop ::= ‘-’ | not | ‘#’ | ‘~’
```

### 消除左递归

表达式 = 表达式 + 数字 ❎

表达式 = 数字 + 表达式 ✅

```ebnf
A := A α | β
变为
A := β A2
A2 := α A2 | 无
```

## Bytecode (lua)

```c
/*===========================================================================
  We assume that instructions are unsigned 32-bit integers.
  All instructions have an opcode in the first 7 bits.
  Instructions can have the following formats:

        3 3 2 2 2 2 2 2 2 2 2 2 1 1 1 1 1 1 1 1 1 1 0 0 0 0 0 0 0 0 0 0
        1 0 9 8 7 6 5 4 3 2 1 0 9 8 7 6 5 4 3 2 1 0 9 8 7 6 5 4 3 2 1 0
iABC          C(8)     |      B(8)     |k|     A(8)      |   Op(7)     |
iABx                Bx(17)               |     A(8)      |   Op(7)     |
iAsBx              sBx (signed)(17)      |     A(8)      |   Op(7)     |
iAx                           Ax(25)                     |   Op(7)     |
isJ                           sJ (signed)(25)            |   Op(7)     |

  A signed argument is represented in excess K: the represented value is
  the written unsigned value minus K, where K is half the maximum for the
  corresponding unsigned argument.
===========================================================================*/

/*
** masks for instruction properties. The format is:
** bits 0-2: op mode
** bit 3: instruction set register A
** bit 4: operator is a test (next instruction must be a jump)
** bit 5: instruction uses 'L->top' set by previous instruction (when B == 0)
** bit 6: instruction sets 'L->top' for next instruction (when C == 0)
** bit 7: instruction is an MM instruction (call a meta_method)
*/
```

### Instructions

我特地写了个宏来生成这些

## Runtime Tables

- Constants Table
- Global Variables Table

## Virtual Machine

Register-based virtual machine,
registers are infinite technologically, I used `Vec` to simulate them.

## Tailcall

Rust doesn't support naive `tailcall`, so I'm planning to implement it by writing a macro for it;

## Closure & Function

Closure is a function with upvalues;

Notice: Index starts from zero (0), different from `lua`

### Upvalue

A **local** variable used by an inner function is called an upvalue (or external local variable, or simply external variable) inside the inner function.

Upvalues have two states:

- Open: Variable is still in its place
- Closed: Closure took the variable
