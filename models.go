package main

import (
	"github.com/kamva/mgm/v3"
	"go.mongodb.org/mongo-driver/bson"
)

type AuthorizedChats struct {
	mgm.DefaultModel `bson:",inline"`
	ChatId           int64 `bson:"chat_id"`
}

var upsert = true

func AuthorizeChat(chatID int64) *AuthorizedChats {
	return &AuthorizedChats{
		ChatId: chatID,
	}
}

func isAuthorized(chatID int64) bool {
	authChat := mgm.Coll(&AuthorizedChats{}).First(bson.M{"chat_id": chatID}, &AuthorizedChats{})
	if authChat != nil {
		return false
	}
	return true
}
