package main

import (
	"log"
	"os"
	"sync"
	"time"

	tg "gopkg.in/telebot.v4"
)

var lock = &sync.Mutex{}

var bot *tg.Bot
var err error

func Bot() *tg.Bot {
	if bot == nil {
		lock.Lock()
		defer lock.Unlock()
		if bot == nil {
			pref := tg.Settings{
				Token:  os.Getenv("BOT_TOKEN"),
				Poller: &tg.LongPoller{Timeout: 10 * time.Second},
			}

			bot, err = tg.NewBot(pref)
			if err != nil {
				log.Fatal(err)
			}
			b := bot.Me
			log.Println("Bot is now running.  Press CTRL-C to exit.", b.FirstName+b.LastName)
		}
	}
	return bot
}
