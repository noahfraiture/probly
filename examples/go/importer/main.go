package main

import (
	"fmt"
	"log"

	"github.com/noahfraiture/probly/bindings/go/probly"
)

func must(err error) {
	if err != nil {
		log.Fatal(err)
	}
}

func main() {
	sketch, err := probly.NewUltraLogLog(12)
	must(err)
	defer func() { must(sketch.Close()) }()

	for _, value := range []string{"alice", "bob", "carol", "alice", "dave"} {
		must(sketch.AddString(value))
	}

	count, err := sketch.Count()
	must(err)
	fmt.Printf("count estimate: %d\n", count)
}
