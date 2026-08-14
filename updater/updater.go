package updater

import (
	"archive/tar"
	"archive/zip"
	"bytes"
	"compress/gzip"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

type ReleaseAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
}

type ReleaseInfo struct {
	TagName      string         `json:"tag_name"`
	Version      string         `json:"-"`
	HTMLURL      string         `json:"html_url"`
	ReleaseNotes string         `json:"body"`
	Assets       []ReleaseAsset `json:"assets"`
	AssetURL     string         `json:"-"`
	AssetName    string         `json:"-"`
	HasUpdate    bool           `json:"-"`
	NewRepo      string         `json:"-"`
}

// GetExecutablePath returns the real physical path of the running binary, resolving symlinks.
func GetExecutablePath() (string, error) {
	execPath, err := os.Executable()
	if err != nil {
		return "", err
	}
	realPath, err := filepath.EvalSymlinks(execPath)
	if err != nil {
		return execPath, nil
	}
	return realPath, nil
}

// CanUpdate checks whether the executable location can be cleanly written to by the current user.
func CanUpdate() (bool, string, string) {
	realPath, err := GetExecutablePath()
	if err != nil {
		return false, "", fmt.Sprintf("failed to get executable path: %v", err)
	}

	dir := filepath.Dir(realPath)

	// Guard against temp directories or nix store
	if strings.HasPrefix(realPath, os.TempDir()) || strings.Contains(realPath, "/nix/store/") {
		return false, realPath, "running from temporary directory or read-only Nix store"
	}

	// Check directory write permission by creating a test file
	testFile, err := os.CreateTemp(dir, ".halptask-perm-check-*")
	if err != nil {
		return false, realPath, fmt.Sprintf("directory %s is read-only or owned by another user (%v)", dir, err)
	}
	testFileName := testFile.Name()
	_ = testFile.Close()
	_ = os.Remove(testFileName)

	return true, realPath, ""
}

// CompareVersions returns 1 if v2 > v1, 0 if v2 == v1, and -1 if v2 < v1.
func CompareVersions(v1, v2 string) int {
	cleanV1 := strings.TrimPrefix(strings.TrimSpace(v1), "v")
	cleanV2 := strings.TrimPrefix(strings.TrimSpace(v2), "v")

	// Separate pre-release info if any
	parts1 := strings.Split(cleanV1, "-")
	parts2 := strings.Split(cleanV2, "-")

	nums1 := strings.Split(parts1[0], ".")
	nums2 := strings.Split(parts2[0], ".")

	maxLen := len(nums1)
	if len(nums2) > maxLen {
		maxLen = len(nums2)
	}

	for i := 0; i < maxLen; i++ {
		var n1, n2 int
		if i < len(nums1) {
			n1, _ = strconv.Atoi(nums1[i])
		}
		if i < len(nums2) {
			n2, _ = strconv.Atoi(nums2[i])
		}
		if n2 > n1 {
			return 1
		}
		if n1 > n2 {
			return -1
		}
	}

	return 0
}

// CheckForUpdate queries the GitHub Releases API for the specified repo.
func CheckForUpdate(currentVersion, repo string) (*ReleaseInfo, error) {
	if repo == "" {
		repo = "halpworld/halptask"
	}

	apiUrl := fmt.Sprintf("https://api.github.com/repos/%s/releases/latest", repo)

	client := &http.Client{
		Timeout: 10 * time.Second,
	}

	req, err := http.NewRequest("GET", apiUrl, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", "halptask/"+currentVersion)
	req.Header.Set("Accept", "application/vnd.github.v3+json")

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode == http.StatusNotFound {
		return nil, fmt.Errorf("repository %s or release not found", repo)
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("GitHub API returned status %d", resp.StatusCode)
	}

	// Check if redirected to a different repository
	finalURL := resp.Request.URL.String()
	var newRepo string
	if strings.Contains(finalURL, "/repos/") {
		parts := strings.Split(finalURL, "/repos/")
		if len(parts) > 1 {
			subParts := strings.Split(parts[1], "/releases")
			if len(subParts) > 0 && subParts[0] != repo {
				newRepo = subParts[0]
			}
		}
	}

	var rel ReleaseInfo
	if err := json.NewDecoder(resp.Body).Decode(&rel); err != nil {
		return nil, fmt.Errorf("failed to parse release JSON: %w", err)
	}

	rel.Version = strings.TrimPrefix(rel.TagName, "v")
	rel.NewRepo = newRepo
	rel.HasUpdate = CompareVersions(currentVersion, rel.Version) > 0

	// Match asset for platform
	matchAsset(&rel)

	return &rel, nil
}

// matchAsset selects the best matching asset for runtime.GOOS and runtime.GOARCH.
func matchAsset(rel *ReleaseInfo) {
	goos := runtime.GOOS
	goarch := runtime.GOARCH

	// Common OS representations
	osNames := []string{goos}
	switch goos {
	case "darwin":
		osNames = append(osNames, "Darwin", "macOS", "apple")
	case "linux":
		osNames = append(osNames, "Linux")
	case "windows":
		osNames = append(osNames, "Windows")
	}

	// Common Arch representations
	archNames := []string{goarch}
	switch goarch {
	case "amd64":
		archNames = append(archNames, "x86_64", "64bit")
	case "386":
		archNames = append(archNames, "i386")
	case "arm64":
		archNames = append(archNames, "aarch64")
	}

	for _, asset := range rel.Assets {
		name := asset.Name
		nameLower := strings.ToLower(name)

		osMatch := false
		for _, osName := range osNames {
			if strings.Contains(nameLower, strings.ToLower(osName)) {
				osMatch = true
				break
			}
		}

		archMatch := false
		for _, archName := range archNames {
			if strings.Contains(nameLower, strings.ToLower(archName)) {
				archMatch = true
				break
			}
		}

		if osMatch && archMatch {
			rel.AssetURL = asset.BrowserDownloadURL
			rel.AssetName = asset.Name
			return
		}
	}

	// Fallback: if only 1 asset exists and OS matches
	for _, asset := range rel.Assets {
		nameLower := strings.ToLower(asset.Name)
		for _, osName := range osNames {
			if strings.Contains(nameLower, strings.ToLower(osName)) {
				rel.AssetURL = asset.BrowserDownloadURL
				rel.AssetName = asset.Name
				return
			}
		}
	}
}

// DoUpdate downloads the matching release asset and atomically replaces the executable.
func DoUpdate(rel *ReleaseInfo) error {
	canUpdate, realPath, reason := CanUpdate()
	if !canUpdate {
		return fmt.Errorf("cannot auto-update binary at %s: %s", realPath, reason)
	}

	if rel == nil || rel.AssetURL == "" {
		return fmt.Errorf("no suitable asset found for platform %s/%s", runtime.GOOS, runtime.GOARCH)
	}

	client := &http.Client{Timeout: 60 * time.Second}
	resp, err := client.Get(rel.AssetURL)
	if err != nil {
		return fmt.Errorf("failed to download update: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download server returned status %d", resp.StatusCode)
	}

	downloadData, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("failed to read download content: %w", err)
	}

	binaryBytes, err := extractBinary(rel.AssetName, downloadData)
	if err != nil {
		return fmt.Errorf("failed to extract binary: %w", err)
	}

	// Get permissions of existing executable
	stat, err := os.Stat(realPath)
	mode := os.FileMode(0755)
	if err == nil {
		mode = stat.Mode()
	}

	dir := filepath.Dir(realPath)

	// Write new binary to a temporary file in the same directory
	tempFile, err := os.CreateTemp(dir, ".halptask-new-*")
	if err != nil {
		return fmt.Errorf("failed to create temporary binary file in %s: %w", dir, err)
	}
	tempPath := tempFile.Name()

	defer func() {
		_ = os.Remove(tempPath)
	}()

	if _, err := tempFile.Write(binaryBytes); err != nil {
		_ = tempFile.Close()
		return fmt.Errorf("failed to write new binary contents: %w", err)
	}

	if err := tempFile.Chmod(mode); err != nil {
		_ = tempFile.Close()
		return fmt.Errorf("failed to set binary permissions: %w", err)
	}
	_ = tempFile.Close()

	// Atomic replacement logic
	if runtime.GOOS == "windows" {
		oldPath := realPath + ".old"
		_ = os.Remove(oldPath)
		if err := os.Rename(realPath, oldPath); err != nil {
			return fmt.Errorf("failed to move current executable on Windows: %w", err)
		}
		if err := os.Rename(tempPath, realPath); err != nil {
			_ = os.Rename(oldPath, realPath)
			return fmt.Errorf("failed to replace executable on Windows: %w", err)
		}
		_ = os.Remove(oldPath)
	} else {
		if err := os.Rename(tempPath, realPath); err != nil {
			return fmt.Errorf("failed to replace executable: %w", err)
		}
	}

	return nil
}

// extractBinary extracts the halptask executable binary from an archive or raw binary payload.
func extractBinary(assetName string, data []byte) ([]byte, error) {
	assetLower := strings.ToLower(assetName)

	if strings.HasSuffix(assetLower, ".tar.gz") || strings.HasSuffix(assetLower, ".tgz") {
		gzReader, err := gzip.NewReader(bytes.NewReader(data))
		if err != nil {
			return nil, err
		}
		defer gzReader.Close()

		tarReader := tar.NewReader(gzReader)
		for {
			header, err := tarReader.Next()
			if err == io.EOF {
				break
			}
			if err != nil {
				return nil, err
			}

			baseName := filepath.Base(header.Name)
			if baseName == "halptask" || baseName == "halptask.exe" {
				return io.ReadAll(tarReader)
			}
		}
		return nil, fmt.Errorf("binary 'halptask' not found in archive %s", assetName)
	}

	if strings.HasSuffix(assetLower, ".zip") {
		zipReader, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
		if err != nil {
			return nil, err
		}

		for _, file := range zipReader.File {
			baseName := filepath.Base(file.Name)
			if baseName == "halptask" || baseName == "halptask.exe" {
				rc, err := file.Open()
				if err != nil {
					return nil, err
				}
				defer rc.Close()
				return io.ReadAll(rc)
			}
		}
		return nil, fmt.Errorf("binary 'halptask' not found in zip archive %s", assetName)
	}

	// Assume raw binary download
	return data, nil
}
