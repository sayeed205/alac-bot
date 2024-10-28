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

	if err != nil {
		//panic("ADMIN_ID is not set")
		log.Println(err)
	}

	// Setup the mgm default config
	err := mgm.SetDefaultConfig(nil, os.Getenv("DB_NAME"), options.Client().ApplyURI(os.Getenv("MONGO_URL")))

	_, _, db, _ := mgm.DefaultConfigs()

	log.Println("Connected to database: ", db.Name())
	if err != nil {
		panic(err)
	}
}
