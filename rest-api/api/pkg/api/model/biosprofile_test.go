// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"

	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

func TestAPIBiosProfileRequestValidate(t *testing.T) {
	tests := []struct {
		name      string
		body      string
		expectErr string
	}{
		{
			name: "string, number and boolean attributes are accepted",
			body: `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3", "attributes": {"A": "Enabled", "B": 5, "C": true, "D": -1.5}}`,
		},
		{
			name: "an empty attribute object is accepted",
			body: `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3", "attributes": {}}`,
		},
		{
			name:      "vendor is required",
			body:      `{"vendor": " ", "model": "ThinkSystem SR680a V3", "attributes": {}}`,
			expectErr: "vendor is required",
		},
		{
			name:      "model is required",
			body:      `{"vendor": "Lenovo", "attributes": {}}`,
			expectErr: "model is required",
		},
		{
			name:      "attributes are required",
			body:      `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3"}`,
			expectErr: "attributes is required",
		},
		{
			name:      "null is not a BIOS value",
			body:      `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3", "attributes": {"A": null}}`,
			expectErr: "attribute A must be a string, number or boolean",
		},
		{
			name:      "objects and arrays are not BIOS values",
			body:      `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3", "attributes": {"A": "Enabled", "B": ["x"], "C": {"D": 1}}}`,
			expectErr: "attribute B must be a string, number or boolean",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var request APIBiosProfileRequest
			require.NoError(t, json.Unmarshal([]byte(tt.body), &request))
			err := request.Validate()
			if tt.expectErr == "" {
				assert.NoError(t, err)
				return
			}
			assert.EqualError(t, err, tt.expectErr)
		})
	}
}

func TestAPIBiosProfileRequestToProto(t *testing.T) {
	var request APIBiosProfileRequest
	require.NoError(t, json.Unmarshal([]byte(`{
		"vendor": "Lenovo",
		"model": "ThinkSystem SR680a V3",
		"attributeRegistry": "BiosAttributeRegistry.1.0.0",
		"attributes": {"SystemRecovery_POSTWatchdogTimerValue": 5, "OperatingModes_ChooseOperatingMode": "MaximumPerformance"},
		"revision": "3f2a9c1d7b4e"
	}`), &request))

	coreReq, err := request.ToProto("lenovo-h200")
	require.NoError(t, err)

	assert.Equal(t, "lenovo-h200", coreReq.GetName())
	assert.Equal(t, "Lenovo", coreReq.GetVendor())
	assert.Equal(t, "ThinkSystem SR680a V3", coreReq.GetModel())
	assert.Equal(t, "BiosAttributeRegistry.1.0.0", coreReq.GetAttributeRegistry())
	assert.Equal(t, "3f2a9c1d7b4e", coreReq.GetRevision())
	assert.JSONEq(t, `{"OperatingModes_ChooseOperatingMode": "MaximumPerformance", "SystemRecovery_POSTWatchdogTimerValue": 5}`, coreReq.GetAttributesJson())
}

func TestNewAPIBiosProfileList(t *testing.T) {
	list, err := NewAPIBiosProfileList(&corev1.BiosProfileList{Profiles: []*corev1.BiosProfile{{
		Name:              "lenovo-h200",
		Vendor:            "Lenovo",
		Model:             "ThinkSystem SR680a V3",
		AttributeRegistry: proto.String("BiosAttributeRegistry.1.0.0"),
		AttributesJson:    `{"OperatingModes_ChooseOperatingMode":"MaximumPerformance"}`,
		Revision:          "3f2a9c1d7b4e",
		UpdatedAt:         timestamppb.New(time.Date(2026, 10, 26, 4, 0, 0, 0, time.UTC)),
	}}})
	require.NoError(t, err)
	encoded, err := json.Marshal(list)
	require.NoError(t, err)
	assert.JSONEq(t, `{"profiles": [{
		"name": "lenovo-h200",
		"vendor": "Lenovo",
		"model": "ThinkSystem SR680a V3",
		"attributeRegistry": "BiosAttributeRegistry.1.0.0",
		"attributes": {"OperatingModes_ChooseOperatingMode": "MaximumPerformance"},
		"revision": "3f2a9c1d7b4e",
		"updatedAt": "2026-10-26T04:00:00Z"
	}]}`, string(encoded))

	empty, err := NewAPIBiosProfileList(&corev1.BiosProfileList{})
	require.NoError(t, err)
	encoded, err = json.Marshal(empty)
	require.NoError(t, err)
	assert.JSONEq(t, `{"profiles": []}`, string(encoded))

	_, err = NewAPIBiosProfileList(&corev1.BiosProfileList{Profiles: []*corev1.BiosProfile{{AttributesJson: `[]`}}})
	assert.Error(t, err)
}
