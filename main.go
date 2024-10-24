package main

import (
	"alac-bot/wrapper"
	"fmt"
	"github.com/kamva/mgm/v3"
	"go.mongodb.org/mongo-driver/bson"
	"log"
	"strconv"

	tg "gopkg.in/telebot.v4"
)

func main() {
	b := Bot()

	b.Handle("/start", func(c tg.Context) error {
		return c.Send("Welcome to apple bot. use /help to see how to use it.\n\nFor now this bot only works in permitted group.", "Markdown")
	})

	b.Handle("/help", func(ctx tg.Context) error {
		return ctx.Send("Available commands - \n/help - Get this message \n/song - Download a single song \n/album - Get album URLs \n/playlist - Get playlist URLs\n\nExample - \n`/song https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359`\n`/song https://music.apple.com/in/song/never-gonna-give-you-up/1559523359`\n\n `/album https://music.apple.com/in/album/3-originals/1559523357`\n\n`/playlist https://music.apple.com/library/playlist/p.vMO5kRQiX1xGMr`", "Markdown")
	})

	b.Handle("/authorize", func(ctx tg.Context) error {
		senderId := ctx.Sender().ID
		if senderId != adminId {
			return nil
		}
		args := ctx.Args()
		var err error
		if len(args) == 0 {
			id := ctx.Chat().ID
			replyTo := ctx.Message().ReplyTo
			if replyTo != nil {
				id = replyTo.Sender.ID
			}
			newChat := AuthorizeChat(id)
			err = mgm.Coll(newChat).First(bson.M{"chat_id": id}, newChat)
			if err != nil {
				log.Println("Authorizing :", id)
				err = mgm.Coll(newChat).Create(newChat)
			}
		} else {
			for _, arg := range args {
				id, err := strconv.ParseInt(arg, 10, 64)
				if err != nil {
					return err
				}
				newChat := AuthorizeChat(id)
				er := mgm.Coll(newChat).First(bson.M{"chat_id": id}, newChat)
				if er != nil {
					log.Println("Authorizing :", id)
					er := mgm.Coll(newChat).Create(newChat)
					if er != nil {
						err = er
						break
					}
				}
				if newChat != nil {
					continue
				}
			}
		}
		if err != nil {
			er := ctx.Reply("Error occurred while authorizing")
			if er != nil {
				return er
			}
			return err
		}

		return ctx.Reply("Authorized successfully")
	})

	b.Handle("/song", func(ctx tg.Context) error {
		chatId := ctx.Chat().ID
		senderId := ctx.Sender().ID
		if !isAuthorized(chatId) || !isAuthorized(senderId) || senderId != adminId {
			return nil
		}

		args := ctx.Args()

		if len(args) > 1 {
			return ctx.Send("Too many arguments!")
		} else if len(args) == 0 {
			return ctx.Send("No url detected!")
		}

		isValid := validateSongUrl(args[0])
		if !isValid {
			return ctx.Send("Invalid URL!")
		}

		urlMeta, err := wrapper.ExtractUrlMeta(args[0])
		if err != nil {
			return err
		}

		file := getFile("song", urlMeta.ID)

		if file != nil {
			return ctx.Reply(&tg.Audio{File: tg.File{FileID: file.FileIds[0]}})
		}

		ID := fmt.Sprintf("%d:%d:%d", ctx.Chat().ID, ctx.Sender().ID, ctx.Message().ID)
		queue = append(queue, CommandRequest{"song", ctx, ID})
		position := len(queue)

		if position > 1 {
			err := sendTempMsg(ctx, fmt.Sprintf("Your request has been queued and you are in position %d.", position), 0)
			if err != nil {
				return err
			}
		}

		commandQueue <- CommandRequest{"song", ctx, ID}
		return nil
	})

	b.Handle("/album", func(ctx tg.Context) error {
		return ctx.Reply("Album support is WIP.")
	})

	b.Handle("/playlist", func(ctx tg.Context) error {
		return ctx.Reply("Playlist support is WIP.")
	})

	b.Handle("/artist", func(ctx tg.Context) error {
		return ctx.Reply("Artist support is WIP.")
	})

	b.Handle("/check", func(ctx tg.Context) error {
		reply := ctx.Message().ReplyTo
		if reply == nil {
			return sendTempMsg(ctx, "Please tag a initiator", 10)
		}

		ID := fmt.Sprintf("%d:%d:%d", ctx.Chat().ID, ctx.Sender().ID, reply.ID)

		var position int
		for i, req := range queue {
			if req.ID == ID {
				position = i + 1
				break
			}
		}

		if position == 1 {
			return sendTempMsg(ctx, "Your request is being processed.", 10)
		} else if position > 0 {
			return sendTempMsg(ctx, fmt.Sprintf("You are in position %d in the queue.", position), 10)

		} else {
			return sendTempMsg(ctx, "Your request is not in the queue or already processed.", 10)
		}
	})

	b.Start()
}
