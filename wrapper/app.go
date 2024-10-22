package wrapper

import (
	"fmt"
	"github.com/abema/go-mp4"
	tg "gopkg.in/telebot.v4"
	"os"
	"regexp"
	"strings"
)

var (
	forbiddenNames = regexp.MustCompile(`[/\\<>:"|?*]`)
)

func App(urlStr string, bot *tg.Bot, ctx tg.Context) error {
	authToken, err := GetToken()
	if err != nil {
		fmt.Println("Error getting auth token :", err)
		return err
	}
	//urlStr := "https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359"
	//userToken := os.Getenv("MEDIA_USER_TOKEN")

	msg, err := bot.Send(ctx.Sender(), "Getting information...", &tg.SendOptions{ReplyTo: ctx.Message().ReplyTo})
	if err != nil {
		return err
	}

	meta, err := GetSongMeta(urlStr, authToken)
	if err != nil {
		fmt.Println("Error getting song metadata :", err)
		return err
	}

	msg, err = bot.Edit(msg, "Found information")
	if err != nil {
		return err
	}

	if meta.Attributes.ExtendedAssetUrls["enhancedHls"] != "" {
		enhancedHls, err := GetEnhanceHls(meta.ID)
		if err != nil {
			fmt.Println(err)
		}
		if strings.HasSuffix(enhancedHls, "m3u8") {
			meta.Attributes.ExtendedAssetUrls["enhancedHls"] = enhancedHls
		}
	}
	songName := fmt.Sprintf("%s - %s", meta.Attributes.Name, meta.Attributes.ArtistName)
	songName = fmt.Sprintf("%s.m4a", forbiddenNames.ReplaceAllString(songName, "_"))

	//lrc := ""
	//if userToken != "" {
	//	lrc, err = GetLyrics(urlStr, authToken)
	//	if err != nil {
	//		fmt.Printf("Failed to parse lyrics: %s \n", err)
	//	}
	//}

	// todo)) check if file already exists
	msg, err = bot.Edit(msg, "Extracting "+songName)
	if err != nil {
		return err
	}

	trackUrl, keys, err := ExtractMedia(meta.Attributes.ExtendedAssetUrls["enhancedHls"])
	if err != nil {
		fmt.Println("\u26A0 Failed to extract info from manifest:", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract info from manifest:")
		return err
	}

	info, err := extractSong(trackUrl)
	if err != nil {
		fmt.Println("Failed to extract track.", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract track.")
		return err
	}

	msg, err = bot.Edit(msg, "Extracted "+songName+"...")
	if err != nil {
		return err
	}

	samplesOk := true
	for samplesOk {
		var totalSize int64 = 0
		for _, i := range info.samples {
			totalSize += int64(len(i.data))
			if int(i.descIndex) >= len(keys) {
				fmt.Println("Decryption size mismatch.")
				samplesOk = false
			}
		}
		info.totalDataSize = totalSize
		break
	}
	if !samplesOk {
		return err
	}

	msg, err = bot.Edit(msg, "Decrypting "+songName+"...")
	decrypted, err := decryptSong(info, keys, meta, bot, ctx, msg)

	if err != nil {
		fmt.Println("Failed to decrypt song.", err)
		return err
	}

	create, err := os.Create(songName)
	if err != nil {
		fmt.Println(err)
		return err
	}
	defer create.Close()

	err = WriteM4a(mp4.NewWriter(create), info, meta, decrypted)
	if err != nil {
		fmt.Println("Failed to write m4a.", err)
		return err
	}
	msg, err = bot.Edit(msg, "Uploading "+songName)
	song := &tg.Audio{
		File:      tg.FromDisk(songName),
		Duration:  meta.Attributes.DurationInMillis / 1000,
		Title:     meta.Attributes.Name,
		Performer: meta.Attributes.ArtistName,
		FileName:  songName,
	}
	err = ctx.Reply(song)
	if err != nil {
		fmt.Println("Failed to upload song.", err)
		_, _ = bot.Send(ctx.Sender(), "Failed to upload song")
		return err
	}
	_ = bot.Delete(msg)
	return nil
}
