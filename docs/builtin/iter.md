

# iter.`range()`
<blockquote>
Create an iterator over a range [from, to)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `from` | int | *false* | *false* | ***required*** | - |
| `to` | int | *false* | *false* | ***required*** | - |
| `step` | int | *false* | *true* | **`1`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# iter.`map()`
<blockquote>
Map each element of an iterable through a function, lazily
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `coll` | any | *false* | *false* | ***required*** | - |
| `f` | function | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# iter.`filter()`
<blockquote>
Keep elements for which pred returns truthy, lazily
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `coll` | any | *false* | *false* | ***required*** | - |
| `pred` | function | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# iter.`take()`
<blockquote>
Take at most n elements from an iterable, lazily
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `coll` | any | *false* | *false* | ***required*** | - |
| `n` | int | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# iter.`to_array()`
<blockquote>
Collect all elements of an iterable into an array
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `coll` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | array |




