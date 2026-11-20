#!/usr/bin/env node
const isNumber = require("is-number");

console.log(isNumber(42) ? "hello from crucible" : "is-number is broken");
