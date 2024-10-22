package main

import (
	"alac-bot/wrapper"
	"fmt"
	"regexp"

	tg "gopkg.in/telebot.v4"
)

func DownloadSong(b *tg.Bot, ctx tg.Context) error {

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

	fmt.Println(args)
	err := wrapper.App(args[0], b, ctx)

	//_, err := b.Send(user, args[0])

	//_, err = b.Edit(msg, "edited the message")

	if err != nil {
		fmt.Println("Error in wrapper", err)
	}
	return err
}

func validateSongUrl(url string) bool {
	// Regular expression for album URLs with an optional 'i' query parameter and other query params
	albumURLRegex := regexp.MustCompile(`^https://music\.apple\.com/([a-z]{2})/album/[a-zA-Z0-9\-]+/([0-9]+)(\?i=([0-9]+).*)?$`)
	// Regular expression for song URLs with optional query params
	songURLRegex := regexp.MustCompile(`^https://music\.apple\.com/([a-z]{2})/song/[a-zA-Z0-9\-]+/([0-9]+)(\?.*)?$`)

	// Check if the URL matches either the album or song pattern
	if albumURLRegex.MatchString(url) || songURLRegex.MatchString(url) {
		return true
	}

	// If no match, return false
	return false
}

func validateAlbumUrl(url string) bool {
	albumURLRegex := regexp.MustCompile(`^https://music\.apple\.com/([a-z]{2})/album/[a-zA-Z0-9\-]+/([0-9]+)(\?.*)?$`)

	// Check if the URL matches the album pattern
	return albumURLRegex.MatchString(url)
}
