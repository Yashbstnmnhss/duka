---
title: UI protocol
description: DUIP, the JSON format between duka and a renderer.
---

DUIP is the format of data based on JSON between the duka end and a renderer end
(usually javascript in web).

- duka -> renderer: **commands** in JSON
- renderer -> duka: **event data** in JSON

## Commands

An array of commands, each with an operator and operands.

| Operator | Description                                            | Operands  |
| :------: | ------------------------------------------------------ | --------- |
|  mount   | Mount to a container by selector (e.g. a DOM element)  | selector  |
|  render  | Render virtual nodes                                   | vnode     |
|  patch   | Diff update                                            | new nodes |
| unmount  | Unmount from container                                 | ~         |
|   log    | For debug trace                                        | message   |

## VNode

|    Field    |  Type   | Description                                                       |
| :---------: | :-----: | ----------------------------------------------------------------- |
|     tag     | string  | Element tag name                                                  |
|     key     | string? | Key for reconciliation (optional)                                 |
| props.class | string  | Element's class name                                              |
| props.style | object? | Element's inline style data (optional)                            |
| props.on\*  | string? | For events, an identifier is sent instead of the function itself  |
|  props.\*   |   any   | Any other property for the element                                |
|  children   |  array  | Children of the current element, text(string) or elements         |

## Events

|  Field  |  Type  | Description                     |
| :-----: | :----: | ------------------------------- |
|  type   | string | Type of event                   |
| target  | string | Target element's selector       |
|  value  |  any   | Value attached to current event |

The javascript side lives in `web/`, an example module in
`example/modules/duka-ui`.
