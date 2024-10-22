package wrapper

import (
	"errors"
	"io"
	"net/http"
	"regexp"
)

func GetToken() (string, error) {
	client := &http.Client{}

	// Step 1: Fetch the main page to find the JS file
	mainPageURL := "https://beta.music.apple.com"
	req, err := http.NewRequest("GET", mainPageURL, nil)
	if err != nil {
		return "", err
	}

	resp, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	// Read the response body
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}

	// Find the index-legacy JS URI using regex
	regex := regexp.MustCompile(`/assets/index-legacy-[^/]+\.js`)
	indexJsUri := regex.FindString(string(body))
	if indexJsUri == "" {
		return "", errors.New("index JS file not found")
	}

	// Step 2: Fetch the JS file to extract the token
	jsFileURL := mainPageURL + indexJsUri
	req, err = http.NewRequest("GET", jsFileURL, nil)
	if err != nil {
		return "", err
	}

	resp, err = client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	// Read the JS file content
	body, err = io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}

	// Extract the token using regex
	regex = regexp.MustCompile(`eyJh[^"]+`)
	token := regex.FindString(string(body))
	if token == "" {
		return "", errors.New("token not found in JS file")
	}

	return token, nil
}
