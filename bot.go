package main

import (
	"log"
	"os"
	"time"

	tg "gopkg.in/telebot.v4"
)

func Bot() *tg.Bot {
	pref := tg.Settings{
		Token:  os.Getenv("BOT_TOKEN"),
		Poller: &tg.LongPoller{Timeout: 10 * time.Second},
	}

	b, err := tg.NewBot(pref)
	if err != nil {
		log.Fatal(err)
	}

	b.Handle("/start", func(c tg.Context) error {
		return c.Send("Welcome to apple bot. use /help to see how to use it.\n\nFor now this bot only works in permitted group.", "Markdown")
	})

	b.Handle("/help", func(ctx tg.Context) error {
		return ctx.Send("Available commands - \n/help - Get this message \n/song - Download a single song \n/album - Get album URLs \n/playlist - Get playlist URLs\n\nExample - \n`/song https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359`\n`/song https://music.apple.com/in/song/never-gonna-give-you-up/1559523359`\n\n `/album https://music.apple.com/in/album/3-originals/1559523357`\n\n`/playlist https://music.apple.com/library/playlist/p.vMO5kRQiX1xGMr`", "Markdown")
	})

	b.Handle("/song", func(ctx tg.Context) error {
		return DownloadSong(b, ctx)
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

	return b
}
