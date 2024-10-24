package main

import (
	"github.com/kamva/mgm/v3"
	"go.mongodb.org/mongo-driver/bson"
)

// <!-------- CHATS --------->

type AuthorizedChats struct {
	mgm.DefaultModel `bson:",inline"`
	ChatId           int64 `bson:"chat_id"`
}

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

// <!-------- FILES --------->

type FileType string

const (
	Song   FileType = "song"
	Album  FileType = "album"
	artist FileType = "artist"
)

type Files struct {
	mgm.DefaultModel `bson:",inline"`
	FileIds          []string `bson:"file_ids"`
	TypeId           string   `bson:"type_id"`
	Type             FileType `bson:"type"`
}

func CreateFile(ids []string, Type FileType, typeId string) *Files {
	return &Files{
		FileIds: ids,
		TypeId:  typeId,
		Type:    Type,
	}
}

func getFile(Type string, typeId string) *Files {
	file := &Files{}
	err = mgm.Coll(file).First(bson.M{"type_id": typeId, "type": Type}, file)
	if err != nil {
		return nil
	}
	return file
}
