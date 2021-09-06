# CW-Vest

This contract allows you to establish a schedule of payments of both native and cw20 tokens.
This sequence is immutable once the contract is instantiated. Any caller can call the Pay function and
trigger the contract to trigger all unpaid payments that are passed their vest date.
