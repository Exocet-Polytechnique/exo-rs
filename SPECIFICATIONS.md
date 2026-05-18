# Spécifications de Module — Cockpit

> **Comment utiliser ce document**
> Ce gabarit doit être rempli afin d'avoir meilleure vue d'ensemble du domaine.
> Il n'est pas nécessaire de savoir comment le module sera codé — concentrez-vous sur *ce que*
> le module doit faire, *ce qu'il* mesure, et *comment* il doit se comporter. Laissez les champs
> inconnus vides et signalez-les à la Section 7.

---

## 1. Identification

| Champ | Valeur |
|---|---|
| **Nom du module** | Cockpit |
| **Auteur de la spec** | Diego |
| **Statut** | `Brouillon` |
| **Dernière mise à jour** | 14/05/26|
| **Version** | v0.1 |

---

## 2. Objectif
Le module cockpit est en charge de recevoir les actions manuelles du pilote pour démarrer et arrêter le catamaran. Il gère la séquence d'allumage, et indique au pilote l'état global du système via des indicateurs lumineux (DEL), et transmet les commandes entrées au reste de l'architecture. Il s'agit de la seule interface pour les entrées du pilote, donc il sera essentiel de penser à toutes les actions qui pourraient être transmises au système. 

---

## 3. Schéma / croquis (optionnel)
*(Joindre une photo, un dessin ou un lien vers un schéma si disponible)*

---

## 4. Données — Ce que le module mesure

Ce module est principalement un module de contrôle et d'affichage. Il ne contient aucun capteur externe à ma connaissance. Des mesures internes potentielles sont la tension d'alimentation et la température du MCU (même si pas très précis). 

| Nom de la variable | Description | Unité | Plage attendue | Fréquence de mise à jour | Source |
|---|---|---|---|---|---|
| `btn_start` | État du bouton de démarrage | Binaire (0/1) | 0 ou 1 | Sur événement | GPIO |
| `btn_stop` | État du bouton d'arrêt | Binaire (0/1) | 0 ou 1 | Sur événement | GPIO |
| `vdd_supply` | Tension d'alimentation du STM32 | V | 3.0 – 3.6V | 1 Hz | Interne au chip |
| `temp_mcu` | Température interne du MCU | °C | -40 – 125°C | 1 Hz | Interne au chip|

> **Note :** `temp_mcu` représente la température du die du STM32, non la température ambiante. Utilisée à des fins de diagnostic uniquement.

## 5. Interface CAN

### 5.1 — Messages que le module **reçoit**

*Quelles données ce module doit-il lire sur le bus CAN ?*

| ID CAN | Nom du signal | Description | Type | Unité | Fréquence |
|---|---|---|---|---|---|
| | | | | | |

### 5.2 — Messages que le module **envoie**

*Quelles données ce module publie-t-il sur le bus CAN ?*

| ID CAN | Nom du signal | Description | Type | Unité | Fréquence |
|---|---|---|---|---|---|
| | | | | | |

> Si les ID CAN ne sont pas encore assignés, laissez la colonne vide et signalez-le à la Section 7.

---

## 6. Processus & Comportements

*Décrivez en langage simple ce que le module doit faire —
sa logique, ses règles et ses décisions. Utilisez des descriptions simples de type « si / alors » ou étape par étape.*

### 6.1 — Fonctionnement normal

*En langage courant, que fait le module dans des conditions normales, du démarrage à l'arrêt ?*

- Étape 1 :
- Étape 2 :
- Étape 3 :

### 6.2 — États de fonctionnement

Voici les états proposés pour le module

| État | Description | Condition d'entrée | Condition de sortie |
|---|---|---|---|
| IDLE | État de repos du bateau <br>- Allumage DEL rouge | État initial par défaut (système inactif initialement) ou après l'état SHUTDOWN | Bouton vert appuyé -> STARTING |
| STARTING | Démarrage du bateau. Comprends:  <br>- Tentative de communication CAN avec le Pi (master)<br>- Ouverture et fermeture des valves<br>- Allumage écran et Pi<br>- Allumage DEL jaune | Bouton vert appuyé | Vérifications complétées avec succès -> ACTIVE |
| ACTIVE | Bateau prêt à opérer. Système en marche. <br>- Allumage DEL verte | Vérifications complétées | Bouton rouge appuyé -> SHUTDOWN |
| SHUTDOWN | Arrêt du bateau <br>- Allumage DEL jaune <br>- Ouverture et fermeture des valves<br>- Fermeture de l'écran et du Pi<br>   | Bouton rouge appuyé | Séquence d'arrêt complétée -> IDLE |
| FAULT | Erreur système <br>- Clignotement DEL rouge  | Échec lors des vérifications ou perte de communication (peut arriver en tout état) | Redémarrage ou intervention manuelle (kill-switch). À voir... |
---

## 7. Comportements en cas de défaut & sécurité

*Que doit faire le module en cas de problème ? Pensez aux pannes de capteurs, aux pertes de communication, aux valeurs hors plage.*

| Scénario de défaut | Comportement attendu | Sévérité |
|---|---|---|
| Séance de vérification non réussite | Mise en état à FAULT | `Critique` |
| Timeout CAN (aucun message de réponse de la part du Rp Pi pendant une limite de temps) | Mise en état à FAULT | `Critique` |

**Contraintes de sécurité (limites absolues qui ne doivent jamais être franchies) :**

- Timeout de 10 secondes de réponse maximum (peut négocier)

---

## 8. Questions ouvertes & dépendances en attente

*Listez tout ce qui est encore inconnu ou en attente d'une autre équipe. C'est ce qui bloque
l'implémentation du module. Soyez précis.*

| # | Question / Dépendance | En attente de | Urgence |
|---|---|---|---|
| 1 | | | `Haute` / `Moyenne` / `Basse` |
| 2 | | | |

---

## 9. Critères d'acceptation

*Comment saurons-nous que ce module fonctionne correctement ? Rédigez des conditions
vérifiables — elles seront utilisées lors des tests d'intégration.*

- [ ]
- [ ]
- [ ]

> Exemple : *« La pompe de refroidissement s'active dans les 200 ms suivant le dépassement du seuil de température »*
> Exemple : *« Une trame DÉFAUT est émise si aucune lecture de température n'est reçue pendant plus de 1 s »*

---

## 10. Notes & Références

*Tout autre élément pertinent : fiches techniques, liens, décisions prises, hypothèses formulées.*

-
---

*Gabarit version 1.0 — Exocet*
