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
	left, err := probly.NewUltraLogLog(12)
	must(err)
	defer func() { must(left.Close()) }()

	right, err := probly.NewUltraLogLog(12)
	must(err)
	defer func() { must(right.Close()) }()

	for _, value := range []string{"alice", "bob", "carol", "alice"} {
		must(left.AddString(value))
	}

	for _, value := range []string{"dave", "erin", "carol", "frank"} {
		must(right.AddString(value))
	}

	leftCount, err := left.Count()
	must(err)
	rightCount, err := right.Count()
	must(err)

	fmt.Printf("left estimate:  %d\n", leftCount)
	fmt.Printf("right estimate: %d\n", rightCount)

	must(left.Merge(right))

	unionCount, err := left.Count()
	must(err)
	fmt.Printf("union estimate: %d\n", unionCount)
}
