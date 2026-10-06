# Attributes

- [declare](#declare)
- [inline](#inline)
- [const](#const)
- [close](#close)
- [data-frozen--bool---false](#data-frozen--bool---false)
- [returns](#returns)
- [keywordish](#keywordish)

## @declare

<a id="declare"></a>

Declare types of function, constant, object

## @inline

<a id="inline"></a>

Available for: function 

Hints the generator to make this function **inline** if possible

## @const

<a id="const"></a>

Available for: variable 

Marks a variable to be a constant. This variable will be immutable

## @close

<a id="close"></a>

Available for: variable 

Marks a variable to be closed automatically

## @data(frozen: bool = false)

<a id="data-frozen--bool---false"></a>

Available for: object 

Automatically generate `init()`, `__eq`, `__tostring` based on properties defined

## @returns(...)

<a id="returns"></a>

Available for: function 

Says what the return slots stand for. `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned

## @keywordish

<a id="keywordish"></a>

Available for: function 

The declaration reads as a **keyword** rather than as a name

