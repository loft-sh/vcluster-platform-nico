// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"net/http"
	"testing"
	"time"

	"github.com/labstack/echo/v4"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	authz "github.com/NVIDIA/infra-controller/rest-api/auth/pkg/authorization"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

func testMachineBiosResponse() *corev1.MachineBios {
	biosVersion := "R3X318M"
	registry := "BiosAttributeRegistry.1.0.0"
	revision := "3f2a9c1d7b4e"
	return &corev1.MachineBios{
		Profile: &corev1.MachineBiosProfile{
			Name:           "lenovo-h200",
			AttributesJson: `{"OperatingModes_ChooseOperatingMode":"MaximumPerformance"}`,
			Revision:       &revision,
		},
		Observation: &corev1.MachineBiosObservation{
			ObservedAt:        timestamppb.New(time.Date(2026, 10, 25, 23, 52, 15, 0, time.UTC)),
			ObservedBy:        "refresh",
			BiosVersion:       &biosVersion,
			AttributeRegistry: &registry,
			CurrentJson:       `{"OperatingModes_ChooseOperatingMode":"CustomMode","SystemRecovery_POSTWatchdogTimerValue":5}`,
			PendingJson:       `{}`,
			AppliedRevision:   &revision,
		},
	}
}

func TestMachineBiosHandlersProxyToCore(t *testing.T) {
	lowerBound := 5.0
	upperBound := 20.0
	defaultValue := "5"
	tests := []struct {
		name       string
		method     string
		fullMethod string
		response   proto.Message
		handler    func(common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc
		expectJSON func(machineID string) string
	}{
		{
			name:       "get returns the stored observation and profile",
			method:     http.MethodGet,
			fullMethod: corev1.Forge_GetMachineBios_FullMethodName,
			response:   testMachineBiosResponse(),
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewGetMachineBiosHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectJSON: func(machineID string) string {
				return `{
					"machineId": "` + machineID + `",
					"profile": {"name": "lenovo-h200", "revision": "3f2a9c1d7b4e", "attributes": {"OperatingModes_ChooseOperatingMode": "MaximumPerformance"}},
					"observation": {
						"observedAt": "2026-10-25T23:52:15Z",
						"observedBy": "refresh",
						"biosVersion": "R3X318M",
						"attributeRegistry": "BiosAttributeRegistry.1.0.0",
						"settingsAppliedAt": null,
						"appliedRevision": "3f2a9c1d7b4e",
						"current": {"OperatingModes_ChooseOperatingMode": "CustomMode", "SystemRecovery_POSTWatchdogTimerValue": 5},
						"pending": {}
					}
				}`
			},
		},
		{
			name:       "get without an observation or profile returns nulls",
			method:     http.MethodGet,
			fullMethod: corev1.Forge_GetMachineBios_FullMethodName,
			response:   &corev1.MachineBios{},
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewGetMachineBiosHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectJSON: func(machineID string) string {
				return `{"machineId": "` + machineID + `", "profile": null, "observation": null}`
			},
		},
		{
			name:       "refresh returns the fresh observation",
			method:     http.MethodPost,
			fullMethod: corev1.Forge_RefreshMachineBios_FullMethodName,
			response:   testMachineBiosResponse(),
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewRefreshMachineBiosHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectJSON: func(machineID string) string {
				return `{
					"machineId": "` + machineID + `",
					"profile": {"name": "lenovo-h200", "revision": "3f2a9c1d7b4e", "attributes": {"OperatingModes_ChooseOperatingMode": "MaximumPerformance"}},
					"observation": {
						"observedAt": "2026-10-25T23:52:15Z",
						"observedBy": "refresh",
						"biosVersion": "R3X318M",
						"attributeRegistry": "BiosAttributeRegistry.1.0.0",
						"settingsAppliedAt": null,
						"appliedRevision": "3f2a9c1d7b4e",
						"current": {"OperatingModes_ChooseOperatingMode": "CustomMode", "SystemRecovery_POSTWatchdogTimerValue": 5},
						"pending": {}
					}
				}`
			},
		},
		{
			name:       "attributes returns the registry",
			method:     http.MethodGet,
			fullMethod: corev1.Forge_GetMachineBiosAttributes_FullMethodName,
			response: &corev1.MachineBiosAttributes{
				Registry: "BiosAttributeRegistry.1.0.0",
				Attributes: []*corev1.MachineBiosAttribute{
					{
						Name:         "SystemRecovery_POSTWatchdogTimerValue",
						DisplayName:  "POST Watchdog Timer Value",
						Type:         "Integer",
						LowerBound:   &lowerBound,
						UpperBound:   &upperBound,
						DefaultValue: &defaultValue,
					},
					{
						Name:          "SystemRecovery_POSTWatchdogTimer",
						DisplayName:   "POST Watchdog Timer",
						Type:          "Enumeration",
						AllowedValues: []string{"Disabled", "Enabled"},
					},
					{
						Name:        "TrustedComputingGroup_DeviceStatus",
						DisplayName: "Device Status",
						Type:        "String",
						ReadOnly:    true,
					},
				},
			},
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewGetMachineBiosAttributesHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectJSON: func(string) string {
				return `{
					"registry": "BiosAttributeRegistry.1.0.0",
					"attributes": [
						{"name": "SystemRecovery_POSTWatchdogTimerValue", "displayName": "POST Watchdog Timer Value", "type": "Integer", "readOnly": false, "allowedValues": [], "lowerBound": 5, "upperBound": 20, "helpText": null, "defaultValue": "5"},
						{"name": "SystemRecovery_POSTWatchdogTimer", "displayName": "POST Watchdog Timer", "type": "Enumeration", "readOnly": false, "allowedValues": ["Disabled", "Enabled"], "lowerBound": null, "upperBound": null, "helpText": null, "defaultValue": null},
						{"name": "TrustedComputingGroup_DeviceStatus", "displayName": "Device Status", "type": "String", "readOnly": true, "allowedValues": [], "lowerBound": null, "upperBound": null, "helpText": null, "defaultValue": null}
					]
				}`
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			fixture := common.NewTestSetupProviderMachineHandlerFixture(t, tt.response)
			if bios, ok := tt.response.(*corev1.MachineBios); ok {
				bios.MachineId = &corev1.MachineId{Id: fixture.MachineID}
			}

			rec := fixture.Request(t, tt.handler(fixture), tt.method, "/", nil, "")
			require.Equal(t, http.StatusOK, rec.Code, rec.Body.String())
			assert.Equal(t, tt.fullMethod, fixture.ProxiedReq.FullMethod)
			assert.Empty(t, fixture.ProxiedReq.EncryptedSecrets)

			var coreReq corev1.MachineId
			require.NoError(t, protojson.Unmarshal(fixture.ProxiedReq.RequestJSON, &coreReq))
			assert.Equal(t, fixture.MachineID, coreReq.GetId())
			assert.JSONEq(t, tt.expectJSON(fixture.MachineID), rec.Body.String())
		})
	}
}

func TestMachineBiosHandlersRejectRequestsBeforeCore(t *testing.T) {
	handlers := map[string]func(common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc{
		"get": func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
			return NewGetMachineBiosHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
		},
		"refresh": func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
			return NewRefreshMachineBiosHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
		},
		"attributes": func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
			return NewGetMachineBiosAttributesHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
		},
	}
	cases := []struct {
		name   string
		mutate func(*common.TestSetupProviderMachineHandlerFixture)
		expect int
	}{
		{
			name: "provider viewer",
			mutate: func(f *common.TestSetupProviderMachineHandlerFixture) {
				f.User = &cdbm.User{OrgData: cdbm.OrgData{f.Org: cdbm.Org{
					Name:  f.Org,
					Roles: []string{authz.ProviderViewerRole},
				}}}
			},
			expect: http.StatusForbidden,
		},
		{
			name: "unknown machine",
			mutate: func(f *common.TestSetupProviderMachineHandlerFixture) {
				f.MachineID = "missing-machine"
			},
			expect: http.StatusNotFound,
		},
		{
			name: "empty machine ID",
			mutate: func(f *common.TestSetupProviderMachineHandlerFixture) {
				f.MachineID = ""
			},
			expect: http.StatusBadRequest,
		},
	}

	for handlerName, handler := range handlers {
		for _, tc := range cases {
			t.Run(handlerName+"/"+tc.name, func(t *testing.T) {
				fixture := common.NewTestSetupProviderMachineHandlerFixture(t, nil)
				tc.mutate(&fixture)

				rec := fixture.Request(t, handler(fixture), http.MethodGet, "/", nil, "")
				assert.Equal(t, tc.expect, rec.Code)
				assert.Empty(t, fixture.ProxiedReq.FullMethod)
			})
		}
	}
}
