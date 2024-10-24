package wrapper

import (
	"github.com/abema/go-mp4"
	"io"
)

type URLMeta struct {
	Storefront string
	URLType    string
	ID         string
}

type SongResponse struct {
	Data []AutoSong `json:"data"`
}

type AutoSong struct {
	ID            string         `json:"id"`
	Type          string         `json:"type"`
	Href          string         `json:"href"`
	Attributes    SongAttributes `json:"attributes"`
	Relationships Relationships  `json:"relationships"`
}

type SongAttributes struct {
	AlbumName                 string            `json:"albumName"`
	HasTimeSyncedLyrics       bool              `json:"hasTimeSyncedLyrics"`
	GenreNames                []string          `json:"genreNames"`
	TrackNumber               int               `json:"trackNumber"`
	DurationInMillis          int               `json:"durationInMillis"`
	ReleaseDate               string            `json:"releaseDate"`
	IsVocalAttenuationAllowed bool              `json:"isVocalAttenuationAllowed"`
	IsMasteredForItunes       bool              `json:"isMasteredForItunes"`
	ISRC                      string            `json:"isrc"`
	Artwork                   Artwork           `json:"artwork"`
	AudioLocale               string            `json:"audioLocale"`
	ComposerName              string            `json:"composerName"`
	URL                       string            `json:"url"`
	PlayParams                PlayParams        `json:"playParams"`
	DiscNumber                int               `json:"discNumber"`
	IsAppleDigitalMaster      bool              `json:"isAppleDigitalMaster"`
	HasLyrics                 bool              `json:"hasLyrics"`
	AudioTraits               []string          `json:"audioTraits"`
	Name                      string            `json:"name"`
	Previews                  []Preview         `json:"previews"`
	ArtistName                string            `json:"artistName"`
	ExtendedAssetUrls         map[string]string `json:"extendedAssetUrls"`
}

type AlbumAttributes struct {
	Copyright           string     `json:"copyright"`
	GenreNames          []string   `json:"genreNames"`
	ReleaseDate         string     `json:"releaseDate"`
	UPC                 string     `json:"upc"`
	IsMasteredForItunes bool       `json:"isMasteredForItunes"`
	Artwork             Artwork    `json:"artwork"`
	PlayParams          PlayParams `json:"playParams"`
	URL                 string     `json:"url"`
	RecordLabel         string     `json:"recordLabel"`
	TrackCount          int        `json:"trackCount"`
	IsCompilation       bool       `json:"isCompilation"`
	IsPrerelease        bool       `json:"isPrerelease"`
	AudioTraits         []string   `json:"audioTraits"`
	IsSingle            bool       `json:"isSingle"`
	Name                string     `json:"name"`
	ArtistName          string     `json:"artistName"`
	IsComplete          bool       `json:"isComplete"`
}

type PlayParams struct {
	ID   string `json:"id"`
	Kind string `json:"kind"`
}

// Preview represents a preview URL for a song.
type Preview struct {
	URL string `json:"url"`
}

type Artwork struct {
	Width      int    `json:"width"`
	URL        string `json:"url"`
	Height     int    `json:"height"`
	TextColor1 string `json:"textColor1"`
	TextColor2 string `json:"textColor2"`
	TextColor3 string `json:"textColor3"`
	TextColor4 string `json:"textColor4"`
	BgColor    string `json:"bgColor"`
	HasP3      bool   `json:"hasP3"`
}

type RelationshipData struct {
	ID         string           `json:"id"`
	Type       string           `json:"type"`
	Href       string           `json:"href"`
	Attributes *AlbumAttributes `json:"attributes,omitempty"` // Nullable for albums
}

// Relationships contains the relationships for a song.
type Relationships struct {
	Albums  Relationship `json:"albums"`
	Artists Relationship `json:"artists"`
}

// Relationship represents a relationship in the API response.
type Relationship struct {
	Href string             `json:"href"`
	Data []RelationshipData `json:"data"`
}

type SongLyrics struct {
	Data []struct {
		Id         string `json:"id"`
		Type       string `json:"type"`
		Attributes struct {
			Ttml       string `json:"ttml"`
			PlayParams struct {
				Id          string `json:"id"`
				Kind        string `json:"kind"`
				CatalogId   string `json:"catalogId"`
				DisplayType int    `json:"displayType"`
			} `json:"playParams"`
		} `json:"attributes"`
	} `json:"data"`
}

type Alac struct {
	mp4.FullBox `mp4:"extend"`

	FrameLength       uint32 `mp4:"size=32"`
	CompatibleVersion uint8  `mp4:"size=8"`
	BitDepth          uint8  `mp4:"size=8"`
	Pb                uint8  `mp4:"size=8"`
	Mb                uint8  `mp4:"size=8"`
	Kb                uint8  `mp4:"size=8"`
	NumChannels       uint8  `mp4:"size=8"`
	MaxRun            uint16 `mp4:"size=16"`
	MaxFrameBytes     uint32 `mp4:"size=32"`
	AvgBitRate        uint32 `mp4:"size=32"`
	SampleRate        uint32 `mp4:"size=32"`
}

type SampleInfo struct {
	data      []byte
	duration  uint32
	descIndex uint32
}

type SongInfo struct {
	r             io.ReadSeeker
	alacParam     *Alac
	samples       []SampleInfo
	totalDataSize int64
}

type Syllable struct {
	Timestamp uint   `json:"timestamp"`
	Text      string `json:"text"`
	Part      bool   `json:"part"`
}

type Line struct {
	Timestamp      uint       `json:"timestamp"`
	Text           []Syllable `json:"text"`
	Endtime        uint       `json:"endtime"`
	OppositeTurn   bool       `json:"oppositeTurn"`
	Background     bool       `json:"background"`
	BackgroundText []Syllable `json:"backgroundText"`
}

type AppleLyricsResponse struct {
	Type    string `json:"type"`
	Content []Line `json:"content"`
}
