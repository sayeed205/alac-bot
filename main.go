package main

import (
	env "github.com/joho/godotenv"
	"log"
)

func main() {
	if err := env.Load(); err != nil {
		log.Println(".env not found")
	}

	b := Bot()
	b.Start()
}
