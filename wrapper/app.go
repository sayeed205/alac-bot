package wrapper

import (
	eerrors "errors"
	"fmt"
	"github.com/abema/go-mp4"
	tg "gopkg.in/telebot.v4"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var (
	forbiddenNames = regexp.MustCompile(`[/\\<>:"|?*]`)
)

func App(urlStr string, folder string, bot *tg.Bot, ctx tg.Context, msg *tg.Message) (*AutoSong, *os.File, error) {
	authToken, err := GetToken()
	if err != nil {
		fmt.Println("Error getting auth token :", err)
		return nil, nil, err
	}
	//urlStr := "https://music.apple.com/in/album/never-gonna-give-you-up/1559523357?i=1559523359"
	//userToken := os.Getenv("MEDIA_USER_TOKEN")

	meta, err := GetSongMeta(urlStr, authToken)
	if err != nil {
		fmt.Println("Error getting song metadata :", err)
		return nil, nil, err
	}

	msg, err = bot.Edit(msg, "Found information")
	if err != nil {
		return nil, nil, err
	}

	if meta.Attributes.ExtendedAssetUrls["enhancedHls"] == "" {
		return nil, nil, eerrors.New("ALAC not available")
	}
	enhancedHls, err := GetEnhanceHls(meta.ID)
	if err != nil {
		fmt.Println(err)
	}
	if strings.HasSuffix(enhancedHls, "m3u8") {
		meta.Attributes.ExtendedAssetUrls["enhancedHls"] = enhancedHls
	}

	songName := fmt.Sprintf("%d.%d. %s - %s", meta.Attributes.DiscNumber, meta.Attributes.TrackNumber, meta.Attributes.Name, meta.Attributes.ArtistName) // 1.1. Never Go... - Rik As....
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
		return nil, nil, err
	}
	fmt.Println("Extracting ", songName)

	trackUrl, keys, err := ExtractMedia(meta.Attributes.ExtendedAssetUrls["enhancedHls"])
	if err != nil {
		fmt.Println("\u26A0 Failed to extract info from manifest:", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract info from manifest:")
		return nil, nil, err
	}

	info, err := extractSong(trackUrl)
	if err != nil {
		fmt.Println("Failed to extract track.", err)
		_, _ = bot.Edit(msg, "\u26A0 Failed to extract track.")
		return nil, nil, err
	}

	msg, err = bot.Edit(msg, "Extracted "+songName+"...")
	if err != nil {
		return nil, nil, err
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
		return nil, nil, err
	}

	msg, err = bot.Edit(msg, "Decrypting "+meta.Attributes.Name+"...")
	decrypted, err := decryptSong(info, keys, meta, bot, ctx, msg)

	if err != nil {
		fmt.Println("Failed to decrypt song.", err)
		return nil, nil, err
	}

	err = os.MkdirAll(folder, os.ModePerm)
	if err != nil {
		fmt.Println("Failed to create folder:", err)
		return nil, nil, err
	}

	file := filepath.Join(folder, songName)
	create, err := os.Create(file)
	if err != nil {
		fmt.Println("Error creating file :", err)
		return nil, nil, err
	}
	defer create.Close()

	err = WriteM4a(mp4.NewWriter(create), info, meta, decrypted)
	if err != nil {
		fmt.Println("Failed to write m4a.", err)
		return nil, nil, err
	}

	//todo))

	return meta, create, err
}
