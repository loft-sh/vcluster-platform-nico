// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/types/known/timestamppb"

	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

func TestNewAPIMachineBios(t *testing.T) {
	observation := func(current string) *corev1.MachineBiosObservation {
		return &corev1.MachineBiosObservation{
			ObservedAt:        timestamppb.New(time.Date(2026, 10, 25, 23, 50, 0, 0, time.UTC)),
			ObservedBy:        "post_reboot",
			SettingsAppliedAt: timestamppb.New(time.Date(2026, 10, 25, 23, 52, 15, 0, time.UTC)),
			CurrentJson:       current,
			PendingJson:       `{"A":"Enabled"}`,
		}
	}
	tests := []struct {
		name       string
		bios       *corev1.MachineBios
		expectJSON string
		expectErr  bool
	}{
		{
			name: "the machine ID from the URL is used when Core omits it",
			bios: &corev1.MachineBios{Observation: observation(`{"A":"Disabled","B":5}`)},
			expectJSON: `{
				"machineId": "fm100ht-url",
				"profile": null,
				"observation": {
					"observedAt": "2026-10-25T23:50:00Z",
					"observedBy": "post_reboot",
					"biosVersion": null,
					"attributeRegistry": null,
					"settingsAppliedAt": "2026-10-25T23:52:15Z",
					"current": {"A": "Disabled", "B": 5},
					"pending": {"A": "Enabled"}
				}
			}`,
		},
		{
			name:      "attributes that are not a JSON object are rejected",
			bios:      &corev1.MachineBios{Observation: observation(`["A"]`)},
			expectErr: true,
		},
		{
			name: "a profile whose attributes are not JSON is rejected",
			bios: &corev1.MachineBios{Profile: &corev1.MachineBiosProfile{
				Name:           "lenovo-h200",
				AttributesJson: `{"A":`,
			}},
			expectErr: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			apiBios, err := NewAPIMachineBios("fm100ht-url", tt.bios)
			if tt.expectErr {
				assert.Error(t, err)
				return
			}
			require.NoError(t, err)
			encoded, err := json.Marshal(apiBios)
			require.NoError(t, err)
			assert.JSONEq(t, tt.expectJSON, string(encoded))
		})
	}
}
