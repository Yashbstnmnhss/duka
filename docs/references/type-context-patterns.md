# type-context.Patterns


<a id="patterns"></a>

> Pattern constructors

## Contents

[RangePat](#rangepat)

[GtPat](#gtpat)

[GePat](#gepat)

[LtPat](#ltpat)

[LePat](#lepat)

[EqPat](#eqpat)

[NePat](#nepat)

[HasPat](#haspat)

[StartsPat](#startspat)

[EndsPat](#endspat)

[RegexPat](#regexpat)

[LenGePat](#lengepat)

[LenLePat](#lenlepat)

[LenGtPat](#lengtpat)

[LenLtPat](#lenltpat)

[LenEqPat](#leneqpat)

[AndPat](#andpat)

[OrPat](#orpat)

[NotPat](#notpat)

## Members

<a id="rangepat"></a>

### type function `RangePat`(type, type)

> Numbers in range [a, b)

<a id="gtpat"></a>

### type function `GtPat`(type, type)

> Numbers greater than N

<a id="gepat"></a>

### type function `GePat`(type, type)

> Numbers greater than or equal to N

<a id="ltpat"></a>

### type function `LtPat`(type, type)

> Numbers less than N

<a id="lepat"></a>

### type function `LePat`(type, type)

> Numbers less than or equal to N

<a id="eqpat"></a>

### type function `EqPat`(type, type)

> Numbers equal to N

<a id="nepat"></a>

### type function `NePat`(type, type)

> Numbers other than N

<a id="haspat"></a>

### type function `HasPat`(type, type)

> Strings containing PART

<a id="startspat"></a>

### type function `StartsPat`(type, type)

> Strings starting with PART

<a id="endspat"></a>

### type function `EndsPat`(type, type)

> Strings ending with PART

<a id="regexpat"></a>

### type function `RegexPat`(type, type)

> Strings matching REGEX in full

<a id="lengepat"></a>

### type function `LenGePat`(type, type)

> Strings at least N characters long

<a id="lenlepat"></a>

### type function `LenLePat`(type, type)

> Strings at most N characters long

<a id="lengtpat"></a>

### type function `LenGtPat`(type, type)

> Strings longer than N characters

<a id="lenltpat"></a>

### type function `LenLtPat`(type, type)

> Strings shorter than N characters

<a id="leneqpat"></a>

### type function `LenEqPat`(type, type)

> Strings exactly N characters long

<a id="andpat"></a>

### type function `AndPat`(type, type)

> Both patterns hold

<a id="orpat"></a>

### type function `OrPat`(type, type)

> Either pattern holds

<a id="notpat"></a>

### type function `NotPat`(type)

> The pattern does not hold

