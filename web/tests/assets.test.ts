import assert from "node:assert/strict"
import test from "node:test"

import { assetUrl, baseForm, type FormEntry } from "../src/lib/assets.ts"

const forms: FormEntry[] = [
  {
    formId: "00025",
    nationalDex: 25,
    nameEn: "Pikachu",
    nameZh: "皮卡丘",
    formCode: null,
    shiny: false,
    url: "/converted/forms/00025/00025.glb",
    bytes: 1,
    animations: ["Idle"],
  },
  {
    formId: "00025_s",
    nationalDex: 25,
    nameEn: "Pikachu",
    nameZh: "皮卡丘",
    formCode: null,
    shiny: true,
    url: "/converted/forms/00025_s/00025_s.glb",
    bytes: 1,
    animations: ["Idle"],
  },
  {
    formId: "00006_51",
    nationalDex: 6,
    nameEn: "Charizard",
    nameZh: "喷火龙",
    formCode: "51",
    shiny: false,
    url: "/converted/forms/00006_51/00006_51.glb",
    bytes: 1,
    animations: ["Idle"],
  },
  {
    formId: "00006",
    nationalDex: 6,
    nameEn: "Charizard",
    nameZh: "喷火龙",
    formCode: null,
    shiny: false,
    url: "/converted/forms/00006/00006.glb",
    bytes: 1,
    animations: ["Idle"],
  },
]

test("base forme maps only when form code is null and not shiny", () => {
  assert.equal(baseForm(forms, 25, 0)?.formId, "00025")
  assert.equal(baseForm(forms, 6, 0)?.formId, "00006")
  assert.equal(baseForm(forms, 6, 1), undefined)
  assert.equal(baseForm(forms, 0, 0), undefined)
})

test("asset urls stay relative unless a base is configured", () => {
  assert.equal(assetUrl("/converted/forms/00025/00025.glb"), "/converted/forms/00025/00025.glb")
})
