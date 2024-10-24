package main

import (
	env "github.com/joho/godotenv"
	"log"
	"os"
	"strconv"

	"github.com/kamva/mgm/v3"
	"go.mongodb.org/mongo-driver/mongo/options"
)

var adminId int64

func init() {
	if err := env.Load(); err != nil {
		log.Println(".env not found")
	}
	adminId, err = strconv.ParseInt(os.Getenv("ADMIN_ID"), 10, 64)

	log.Println("ADMIN_ID", adminId)
	if err != nil {
		//panic("ADMIN_ID is not set")
		log.Println(err)
	}

	// Setup the mgm default config
	err := mgm.SetDefaultConfig(nil, "alac-bot", options.Client().ApplyURI(os.Getenv("MONGO_URL")))

	if err != nil {
		panic(err)
	}
}
