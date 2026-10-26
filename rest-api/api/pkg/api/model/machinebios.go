// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"encoding/json"
	"fmt"
	"time"

	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

type APIMachineBios struct {
	MachineID   string                     `json:"machineId"`
	Profile     *APIMachineBiosProfile     `json:"profile"`
	Observation *APIMachineBiosObservation `json:"observation"`
}

type APIMachineBiosProfile struct {
	Name       string          `json:"name"`
	Revision   *string         `json:"revision"`
	Attributes json.RawMessage `json:"attributes"`
}

type APIMachineBiosObservation struct {
	ObservedAt        time.Time       `json:"observedAt"`
	ObservedBy        string          `json:"observedBy"`
	BiosVersion       *string         `json:"biosVersion"`
	AttributeRegistry *string         `json:"attributeRegistry"`
	SettingsAppliedAt *time.Time      `json:"settingsAppliedAt"`
	AppliedRevision   *string         `json:"appliedRevision"`
	Current           json.RawMessage `json:"current"`
	Pending           json.RawMessage `json:"pending"`
}

type APIMachineBiosApplyResponse struct {
	Accepted bool `json:"accepted"`
}

type APIMachineBiosAttributes struct {
	Registry   string                    `json:"registry"`
	Attributes []APIMachineBiosAttribute `json:"attributes"`
}

type APIMachineBiosAttribute struct {
	Name          string   `json:"name"`
	DisplayName   string   `json:"displayName"`
	Type          string   `json:"type"`
	ReadOnly      bool     `json:"readOnly"`
	AllowedValues []string `json:"allowedValues"`
	LowerBound    *float64 `json:"lowerBound"`
	UpperBound    *float64 `json:"upperBound"`
	HelpText      *string  `json:"helpText"`
	DefaultValue  *string  `json:"defaultValue"`
}

func NewAPIMachineBios(machineID string, bios *corev1.MachineBios) (*APIMachineBios, error) {
	apiBios := &APIMachineBios{MachineID: machineID}
	if id := bios.GetMachineId().GetId(); id != "" {
		apiBios.MachineID = id
	}

	if profile := bios.GetProfile(); profile != nil {
		attributes, err := biosAttributesJSON("profile attributes", profile.GetAttributesJson())
		if err != nil {
			return nil, err
		}
		apiBios.Profile = &APIMachineBiosProfile{
			Name:       profile.GetName(),
			Revision:   profile.Revision,
			Attributes: attributes,
		}
	}

	if observation := bios.GetObservation(); observation != nil {
		current, err := biosAttributesJSON("current attributes", observation.GetCurrentJson())
		if err != nil {
			return nil, err
		}
		pending, err := biosAttributesJSON("pending attributes", observation.GetPendingJson())
		if err != nil {
			return nil, err
		}
		apiObservation := &APIMachineBiosObservation{
			ObservedBy:        observation.GetObservedBy(),
			BiosVersion:       observation.BiosVersion,
			AttributeRegistry: observation.AttributeRegistry,
			AppliedRevision:   observation.AppliedRevision,
			Current:           current,
			Pending:           pending,
		}
		if observedAt := observation.GetObservedAt(); observedAt != nil {
			apiObservation.ObservedAt = observedAt.AsTime()
		}
		if settingsAppliedAt := observation.GetSettingsAppliedAt(); settingsAppliedAt != nil {
			appliedAt := settingsAppliedAt.AsTime()
			apiObservation.SettingsAppliedAt = &appliedAt
		}
		apiBios.Observation = apiObservation
	}

	return apiBios, nil
}

func NewAPIMachineBiosAttributes(attributes *corev1.MachineBiosAttributes) APIMachineBiosAttributes {
	apiAttributes := APIMachineBiosAttributes{
		Registry:   attributes.GetRegistry(),
		Attributes: []APIMachineBiosAttribute{},
	}
	for _, attribute := range attributes.GetAttributes() {
		apiAttributes.Attributes = append(apiAttributes.Attributes, APIMachineBiosAttribute{
			Name:          attribute.GetName(),
			DisplayName:   attribute.GetDisplayName(),
			Type:          attribute.GetType(),
			ReadOnly:      attribute.GetReadOnly(),
			AllowedValues: append([]string{}, attribute.GetAllowedValues()...),
			LowerBound:    attribute.LowerBound,
			UpperBound:    attribute.UpperBound,
			HelpText:      attribute.HelpText,
			DefaultValue:  attribute.DefaultValue,
		})
	}
	return apiAttributes
}

func biosAttributesJSON(field string, value string) (json.RawMessage, error) {
	var attributes map[string]json.RawMessage
	if err := json.Unmarshal([]byte(value), &attributes); err != nil {
		return nil, fmt.Errorf("BIOS %s are not a JSON object: %w", field, err)
	}
	return json.RawMessage(value), nil
}
