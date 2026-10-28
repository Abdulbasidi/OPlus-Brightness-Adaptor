/*
 * Copyright (C) 2024 The OPlus Software
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package vendor.oplus.hardware.displaypanelfeature;

@VintfStability
interface IDisplayPanelFeature {
    int getDisplayPanelFeatureValue(in int FeatureId, out int[] FeatureValues);
    int setDisplayPanelFeatureValue(in int FeatureId, in int[] FeatureValues);
    int getDisplayPanelInfo(in int FeatureId, out List<String> PanelInfo);
}
