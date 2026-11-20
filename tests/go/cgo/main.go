package main

// static int answer(void) { return 42; }
import "C"

import (
	"fmt"
	"net"
)

func main() {
	// net with cgo on is what links against the system resolver.
	_, _ = net.LookupHost("localhost")
	fmt.Println(int(C.answer()))
}
