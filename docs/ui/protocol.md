# Duka UI Protocol (DUIP)

This file describes the format of data based on JSON between Duka-end and other renderer end (usually JavaScript-end in Web).

## Transmission

- Duka to Renderer: Duka sends **commands** in JSON format
- Renderer to Duka: Renderer returns **event data** to Duka in JSON format

## Data Scheme

### Commands

This is an array of commands with a operator and operands. Commands are listed below:

| Operator | Description                                                | Operands  |
| :------: | :--------------------------------------------------------- | :-------: |
|  mount   | Mount to a container by selector (e.g. DOM element in Web) | selector  |
|  render  | Render virtual nodes                                       |   vnode   |
|  patch   | Diff update                                                | New nodes |
| unmount  | Unmount from container                                     |     ~     |
|   log    | For debug trace                                            |  message  |

### VNode Data

VNode represents virtual node, which describes the composition of UI to be rendered.

|    Field    |  Type   | Description                                                                          |
| :---------: | :-----: | :----------------------------------------------------------------------------------- |
|     tag     | string  | Element tag name                                                                     |
|     key     | string? | Key for reconciliation (optional)                                                    |
| props.class | string  | Element's class name                                                                 |
| props.style | object? | Element's inline style data (optional)                                               |
| props.on\*  | string? | For events, see below. This will transmit an identifier instead of function directly |
|  props.\*   |   any   | Any other property for element                                                       |
|  children   |  array  | Children of current element, supports text(string) and other elements                |

### Event Data

| Field  |  Type  | Description                     |
| :----: | :----: | :------------------------------ |
|  type  | string | Type of event                   |
| target | string | Target element's selector       |
| value  |  any   | Value attached to current event |
