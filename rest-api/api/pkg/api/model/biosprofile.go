// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"

	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

type APIBiosProfileRequest struct {
	Vendor            string                     `json:"vendor"`
	Model             string                     `json:"model"`
	AttributeRegistry *string                    `json:"attributeRegistry"`
	Attributes        map[string]json.RawMessage `json:"attributes"`
	Revision          string                     `json:"revision"`
}

type APIBiosProfile struct {
	Name              string          `json:"name"`
	Vendor            string          `json:"vendor"`
	Model             string          `json:"model"`
	AttributeRegistry *string         `json:"attributeRegistry"`
	Attributes        json.RawMessage `json:"attributes"`
	Revision          string          `json:"revision"`
	UpdatedAt         *time.Time      `json:"updatedAt"`
}

type APIBiosProfileList struct {
	Profiles []APIBiosProfile `json:"profiles"`
}

func (r APIBiosProfileRequest) Validate() error {
	if strings.TrimSpace(r.Vendor) == "" {
		return errors.New("vendor is required")
	}
	if strings.TrimSpace(r.Model) == "" {
		return errors.New("model is required")
	}
	if r.Attributes == nil {
		return errors.New("attributes is required")
	}
	names := make([]string, 0, len(r.Attributes))
	for name := range r.Attributes {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		if !isJSONScalar(r.Attributes[name]) {
			return fmt.Errorf("attribute %s must be a string, number or boolean", name)
		}
	}
	return nil
}

func isJSONScalar(value json.RawMessage) bool {
	trimmed := bytes.TrimSpace(value)
	if len(trimmed) == 0 {
		return false
	}
	switch trimmed[0] {
	case '"', 't', 'f', '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
		return json.Valid(trimmed)
	default:
		return false
	}
}

func (r APIBiosProfileRequest) ToProto(name string) (*corev1.BiosProfile, error) {
	attributes, err := json.Marshal(r.Attributes)
	if err != nil {
		return nil, fmt.Errorf("BIOS profile attributes could not be encoded: %w", err)
	}
	return &corev1.BiosProfile{
		Name:              name,
		Vendor:            r.Vendor,
		Model:             r.Model,
		AttributeRegistry: r.AttributeRegistry,
		AttributesJson:    string(attributes),
		Revision:          r.Revision,
	}, nil
}

func NewAPIBiosProfile(profile *corev1.BiosProfile) (*APIBiosProfile, error) {
	attributes, err := biosAttributesJSON("profile attributes", profile.GetAttributesJson())
	if err != nil {
		return nil, err
	}
	apiProfile := &APIBiosProfile{
		Name:              profile.GetName(),
		Vendor:            profile.GetVendor(),
		Model:             profile.GetModel(),
		AttributeRegistry: profile.AttributeRegistry,
		Attributes:        attributes,
		Revision:          profile.GetRevision(),
	}
	if updatedAt := profile.GetUpdatedAt(); updatedAt != nil {
		updated := updatedAt.AsTime()
		apiProfile.UpdatedAt = &updated
	}
	return apiProfile, nil
}

func NewAPIBiosProfileList(list *corev1.BiosProfileList) (*APIBiosProfileList, error) {
	apiList := &APIBiosProfileList{Profiles: []APIBiosProfile{}}
	for _, profile := range list.GetProfiles() {
		apiProfile, err := NewAPIBiosProfile(profile)
		if err != nil {
			return nil, err
		}
		apiList.Profiles = append(apiList.Profiles, *apiProfile)
	}
	return apiList, nil
}
